//! Typed mutation identity, time authority, and the sole typed-to-byte finalizer.
//!
//! Commands submit timestamp-free semantic [`MutationIntent`]s; a created issue
//! carries the id its caller derived from the context, and every other
//! identifier is the finalizer's.
//! A [`MutationContext`] — created exactly once per command operation, before
//! its conflict-retry loop — supplies deterministic identifiers (from a single seed drawn
//! once on the first allocation) and one mutation timestamp (sampled once, on the
//! first transition), so retries reuse identical record identities and times while
//! a no-op mutation samples neither identity nor time. The
//! finalizer serializes every repository-owned record (issue upserts/deletes,
//! `index.json` membership, gate-run/audit artifacts, and events) into one exact
//! [`RepositoryDelta`] published through one redo record, preserving
//! `@/inv/event-log` and `@/inv/atomic-writes`.
//!
//! This module performs no filesystem I/O and imports only `domain` value types
//! and the pure `repository_state` vocabulary.

use super::{
    CaptureError, DeltaError, ExpectedPreimage, FileMode, MaterializationPlan, RepositoryAction,
    RepositoryActionOwner, RepositoryDelta, RepositoryEntry, RepositoryImage, RepositoryLayout,
    RepositoryLayoutError, RootRelativePath, VirtualPath,
};
use crate::declarations::GateRegistry;
use crate::domain::event_catalog::EventTag;
use crate::domain::identity::IssueId;
use crate::domain::{
    Assignee, Event, EventBody, GateRunResult, GateStatus, Issue, LinkedCheckoutWriteStance, State,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cell::Cell;

/// Injected wall-clock source for the single mutation timestamp.
///
/// Production uses [`SystemMutationClock`]; memory and tests inject a fixed clock
/// so serialized lifecycle timestamps are deterministic and byte-comparable.
pub trait MutationClock: Send + Sync {
    /// The instant applied to every transition that occurs in one mutation.
    fn now(&self) -> DateTime<Utc>;
}

/// Production clock sampling the operating-system wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemMutationClock;

impl MutationClock for SystemMutationClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Deterministic clock returning one fixed instant, for memory and tests.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FixedMutationClock(DateTime<Utc>);

impl FixedMutationClock {
    /// Construct a clock that always returns `instant`.
    pub(crate) fn new(instant: DateTime<Utc>) -> Self {
        Self(instant)
    }
}

impl MutationClock for FixedMutationClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// Deterministic identifier source.
///
/// Production draws one seed from the UUID source lazily, on the first identifier
/// allocation ([`IdAuthority::random`]); memory and tests inject a fixed seed
/// ([`IdAuthority::from_seed`]). Every identifier is derived purely from the seed
/// and a frozen allocation index, so repeated finalization within one attempt or
/// after a captured-image retry reproduces byte-identical values. A
/// no-op mutation allocates no identifier, so a production authority never draws
/// its random source.
#[derive(Debug, Clone)]
pub struct IdAuthority {
    seed: SeedSource,
}

/// The 32-byte identity seed, either injected up front or sampled once on demand.
#[derive(Debug, Clone)]
enum SeedSource {
    /// A fixed seed injected for deterministic memory/test allocation.
    Fixed([u8; 32]),
    /// A production seed drawn from the UUID source at most once, on first use.
    Lazy(Cell<Option<[u8; 32]>>),
}

/// Draw one 32-byte identity seed from a single UUID sample, expanded by SHA-256
/// under a domain-separation tag. The UUID source is sampled exactly once.
fn sample_random_seed() -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"jit-mutation-seed-v1");
    hasher.update(uuid::Uuid::new_v4().as_bytes());
    let digest = hasher.finalize();
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&digest);
    seed
}

impl IdAuthority {
    /// Inject a fixed seed for deterministic memory/test identity allocation.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            seed: SeedSource::Fixed(seed),
        }
    }

    /// A production authority that draws its seed from the UUID source lazily, on
    /// the first identifier allocation, so a no-op mutation samples nothing.
    pub fn random() -> Self {
        Self {
            seed: SeedSource::Lazy(Cell::new(None)),
        }
    }

    /// The seed, drawing the production random source once on first use.
    fn seed(&self) -> [u8; 32] {
        match &self.seed {
            SeedSource::Fixed(seed) => *seed,
            SeedSource::Lazy(cell) => match cell.get() {
                Some(seed) => seed,
                None => {
                    let seed = sample_random_seed();
                    cell.set(Some(seed));
                    seed
                }
            },
        }
    }

    /// The seed if already determined, without drawing the random source.
    #[cfg(test)]
    fn sampled_seed(&self) -> Option<[u8; 32]> {
        match &self.seed {
            SeedSource::Fixed(seed) => Some(*seed),
            SeedSource::Lazy(cell) => cell.get(),
        }
    }

    /// Derive the hyphenated UUID string for allocation `index`.
    ///
    /// Exposed to the crate so an orchestrator can pre-derive the identifiers of
    /// records whose expected-absent paths must enter the `CaptureSpec` before
    /// capture (new issue and gate-run paths), replaying the same frozen order.
    /// The first call on a production authority draws its seed (see [`Self::random`]).
    pub(crate) fn uuid_at(&self, index: u64) -> String {
        self.uuid_value_at(index).to_string()
    }

    /// The UUID for allocation `index`, before rendering.
    fn uuid_value_at(&self, index: u64) -> uuid::Uuid {
        let mut hasher = Sha256::new();
        hasher.update(b"jit-mutation-id-v1");
        hasher.update(self.seed());
        hasher.update(index.to_be_bytes());
        let digest = hasher.finalize();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        // Stamp RFC 4122 version 4 and the DCE variant so the value is a
        // well-formed random-shaped UUID, matching the format callers expect.
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        uuid::Uuid::from_bytes(bytes)
    }
}

/// Per-operation identity and time authority.
///
/// Holds the injected [`IdAuthority`] and [`MutationClock`]. Both the identity
/// seed and the timestamp are sampled once, lazily — the seed on the first
/// identifier allocation, the timestamp on the first transition — so a no-op
/// mutation samples neither. Interior mutability keeps the context shared
/// by `&self` across the finalizer's passes while remaining reusable across
/// retries: [`MutationContext::begin`] restarts the frozen allocation order.
pub struct MutationContext {
    ids: IdAuthority,
    clock: Box<dyn MutationClock>,
    dispatch_annotation: Option<MutationContextAnnotation>,
    sampled: Cell<Option<DateTime<Utc>>>,
    next_index: Cell<u64>,
}

/// One invocation-scoped fact carried from command dispatch into mutation finalization.
///
/// Top-level dispatch decides the fact once and installs it for the rest of the
/// invocation — through
/// [`DispatchMutationAnnotation`](crate::commands::DispatchMutationAnnotation) or its
/// closure form — instead of threading it through every command module. The factory
/// stamps it into every production context it builds, and the finalizer's event pass
/// turns it into an audit record composed into the same plan as the mutation it
/// annotates.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MutationContextAnnotation {
    /// An explicit per-invocation override outranked the repository's declared
    /// linked-checkout write stance and permitted this invocation to mutate the store
    /// inside a linked non-primary checkout.
    LinkedCheckoutWriteOverridden {
        /// Root of the linked checkout whose store the invocation mutates, taken from
        /// the selected-root worktree authority rather than re-detected.
        checkout: std::path::PathBuf,
        /// The stance declared for the repository, which would have refused.
        declared_stance: LinkedCheckoutWriteStance,
        /// The per-invocation stance that outranked the declaration.
        invocation_override: LinkedCheckoutWriteStance,
    },
}

/// Constructs production mutation contexts from one dispatch-scoped annotation slot.
///
/// The command layer owns the single live factory and scopes changes to its annotation.
/// Keeping construction here lets [`MutationContext::production`] remain inaccessible to
/// command modules and integration tests.
#[derive(Default)]
pub(crate) struct MutationContextFactory {
    dispatch_annotation: Option<MutationContextAnnotation>,
}

impl MutationContextFactory {
    pub(crate) fn create(&self) -> MutationContext {
        MutationContext::production(self.dispatch_annotation.clone())
    }

    pub(crate) fn replace_annotation(
        &mut self,
        annotation: Option<MutationContextAnnotation>,
    ) -> Option<MutationContextAnnotation> {
        std::mem::replace(&mut self.dispatch_annotation, annotation)
    }
}

impl MutationContext {
    /// Construct a context from an injected authority and clock.
    pub fn new(ids: IdAuthority, clock: Box<dyn MutationClock>) -> Self {
        Self::with_annotation(ids, clock, None)
    }

    fn with_annotation(
        ids: IdAuthority,
        clock: Box<dyn MutationClock>,
        dispatch_annotation: Option<MutationContextAnnotation>,
    ) -> Self {
        Self {
            ids,
            clock,
            dispatch_annotation,
            sampled: Cell::new(None),
            next_index: Cell::new(0),
        }
    }

    /// Build the factory-owned production context from a lazily-drawn random seed and the
    /// system clock.
    ///
    /// Command code must route through `commands::production_mutation_context`; keeping
    /// this constructor private prevents another production construction convention from
    /// emerging beside that factory.
    ///
    /// A no-op mutation allocates no identifier and stamps no transition, so it
    /// draws neither the random source nor the clock.
    fn production(dispatch_annotation: Option<MutationContextAnnotation>) -> Self {
        Self::with_annotation(
            IdAuthority::random(),
            Box::new(SystemMutationClock),
            dispatch_annotation,
        )
    }

    /// A deterministic context for memory and tests: fixed seed and clock.
    pub fn deterministic(seed: [u8; 32], instant: DateTime<Utc>) -> Self {
        Self::new(
            IdAuthority::from_seed(seed),
            Box::new(FixedMutationClock::new(instant)),
        )
    }

    /// Stable non-publishing context for deterministic previews.
    pub fn preview() -> Self {
        Self::deterministic(
            [0; 32],
            DateTime::<Utc>::from_timestamp(0, 0).expect("the Unix epoch is representable"),
        )
    }

    /// Invocation-scoped annotation captured by the production factory, if any.
    pub fn dispatch_annotation(&self) -> Option<&MutationContextAnnotation> {
        self.dispatch_annotation.as_ref()
    }

    /// Derive the id of the issue created at position `index` of a mutation's
    /// creation order. The finalizer reserves those leading indices, so a draft
    /// built with this id keeps every later identifier in the frozen order.
    pub(crate) fn issue_id_at(&self, index: u64) -> IssueId {
        IssueId::from_uuid(self.ids.uuid_value_at(index))
    }

    /// Derive an identifier at the finalizer's frozen allocation index without
    /// advancing that order. Command orchestration uses this only to capture the
    /// expected-absent paths of gate-run records, whose finalizer-assigned ids
    /// are part of their path, and to return those ids after publication.
    pub(crate) fn identifier_at(&self, index: u64) -> String {
        self.ids.uuid_at(index)
    }

    /// Restart the frozen allocation order for another finalize pass.
    ///
    /// This applies both to probe/final passes within one attempt and to a later
    /// captured-image retry. The operation-scoped context preserves its sampled
    /// seed and timestamp across both cases.
    fn begin(&self) {
        self.next_index.set(0);
    }

    /// Sample the single mutation timestamp on first use; memoized thereafter.
    pub(crate) fn timestamp(&self) -> DateTime<Utc> {
        match self.sampled.get() {
            Some(instant) => instant,
            None => {
                let instant = self.clock.now();
                self.sampled.set(Some(instant));
                instant
            }
        }
    }

    /// Skip the `count` indices the mutation's issue creations derived their
    /// ids at.
    fn reserve(&self, count: usize) {
        self.next_index.set(self.next_index.get() + count as u64);
    }

    /// Allocate the next identifier in the frozen order.
    fn allocate(&self) -> String {
        let index = self.next_index.get();
        self.next_index.set(index + 1);
        self.ids.uuid_at(index)
    }
}

/// One repository-owned semantic mutation, free of timestamps.
///
/// The finalizer applies the mutation timestamp and assigns every identifier
/// except a created issue's, which the caller derives through
/// [`MutationContext::issue_id_at`]; callers never stamp time. The set is
/// closed: a new record class extends this enum.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MutationIntent {
    /// Create a new issue. The finalizer stamps `created_at`, `updated_at`, and
    /// `first_ready_at` (when initially `Ready`); every other field, the id
    /// included, is taken verbatim from `draft`.
    CreateIssue {
        /// Semantic issue whose lifecycle timestamps are stamped by the
        /// finalizer; all other fields are authoritative.
        draft: Box<Issue>,
    },
    /// Create a prevalidated issue batch and wire all intra-batch dependency
    /// relationships in the same delta. Dependency pairs are
    /// `(dependent_index, dependency_index)` into `drafts`; the finalizer
    /// resolves them to the drafts' ids.
    CreateIssueBatch {
        /// Semantic issue drafts in caller request order.
        drafts: Vec<Issue>,
        /// Prevalidated intra-batch dependency index pairs.
        dependencies: Vec<(usize, usize)>,
    },
    /// Convergently claim `issue_id` for `agent`. Idempotent over both the
    /// captured issue assignment and the captured event-log tail: a fully
    /// reflected claim is a complete no-op; an issue assigned without its claim
    /// event emits exactly the missing event; a duplicate event is never emitted.
    ClaimIssue {
        /// Canonical full issue id (the caller resolves short ids first).
        issue_id: IssueId,
        /// Claiming agent identity.
        agent: Assignee,
    },
    /// Semantic full-issue update of an issue that ALREADY EXISTS in the captured
    /// image. The finalizer stamps `updated_at` from the mutation clock and derives
    /// `created_at`/`first_ready_at` from the captured preimage (they are not the
    /// caller's to set), and names in each gate status the run a
    /// [`MutationIntent::RecordGateRun`] of the same mutation records for that
    /// issue and gate; every other field is taken verbatim from `issue`. Index
    /// membership is unchanged (no id is allocated), and a target absent from the
    /// captured image is a typed [`MutationError::MissingIssue`] — never an implicit
    /// create.
    UpdateIssue {
        /// Semantic issue whose id must match a captured issue; its lifecycle
        /// timestamps are reconciled against the preimage by the finalizer.
        issue: Box<Issue>,
    },
    /// Replace the authored gate registry with a structured proposal derived
    /// from the registry in the captured image. The finalizer owns canonical
    /// declaration serialization; callers cannot submit repository bytes.
    EditGateRegistry {
        /// Complete proposed registry after one closed semantic edit.
        registry: Box<GateRegistry>,
    },
    /// Create one project-defined gate preset at its canonical path. The target
    /// must be absent in the captured image, so a preset never overwrites
    /// another.
    CreateGatePreset {
        /// Validated semantic preset whose name determines the filename stem.
        preset: Box<crate::gate_presets::GatePresetDefinition>,
    },
    /// Delete an issue that ALREADY EXISTS in the captured image. The finalizer
    /// removes the issue record and transfers its id from active to deleted index
    /// membership in the same delta. Audit events remain explicit intents so the
    /// caller can compose deletion with dependent-edge repairs atomically.
    DeleteIssue {
        /// Canonical full issue id whose captured record must exist.
        issue_id: IssueId,
    },
    /// Record one gate-run artifact. The finalizer assigns `run_id`, before the
    /// issue updates that name it, and preserves
    /// external checker `started_at`/`completed_at` evidence verbatim; an
    /// attestation, which ran no checker, is stamped with the mutation timestamp.
    RecordGateRun {
        /// Semantic gate-run result whose `run_id` is assigned by the finalizer.
        draft: Box<GateRunResult>,
    },
    /// Append one repository-owned provenance/audit event. The finalizer assigns
    /// the event id and stamps its timestamp.
    RecordEvent {
        /// Deterministic ordering phase within the mutation.
        phase: u8,
        /// Drafted body the finalizer stamps with an id and timestamp.
        event: Box<EventBody>,
    },
}

/// One pending event awaiting deterministic ordering and identity assignment.
struct PendingEvent {
    phase: u8,
    tag: EventTag,
    primary: Option<IssueId>,
    secondary: String,
    ordinal: usize,
    body: EventBody,
}

impl PendingEvent {
    fn sort_key(&self) -> (u8, &str, Option<&IssueId>, &str, usize) {
        (
            self.phase,
            self.tag.as_str(),
            self.primary.as_ref(),
            self.secondary.as_str(),
            self.ordinal,
        )
    }
}

/// Finalizer failure.
#[derive(Debug, thiserror::Error)]
pub enum MutationError {
    /// Reading captured evidence failed.
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// Delta normalization rejected the produced actions.
    #[error(transparent)]
    Delta(#[from] DeltaError),
    /// Canonicalization of a produced target failed.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    /// A record could not be serialized.
    #[error("failed to serialize repository record: {0}")]
    Serialize(#[from] serde_json::Error),
    /// An authored gate registry could not be serialized canonically.
    #[error(transparent)]
    GateDeclaration(#[from] crate::declarations::GateDeclarationError),
    /// A captured record could not be parsed.
    #[error("captured record at {path:?} is malformed: {reason}")]
    MalformedRecord {
        /// The offending path.
        path: VirtualPath,
        /// Stable diagnostic.
        reason: String,
    },
    /// A referenced issue was absent from the captured image.
    #[error("issue {0} is absent from the captured image")]
    MissingIssue(IssueId),
    /// The captured `events.jsonl` ends in an uncertified torn tail and this
    /// mutation carries no profile lifecycle marker to certify it, so appending
    /// would produce a log the reader rejects.
    #[error(
        "captured events.jsonl has an uncertified torn tail and the mutation has no \
         profile lifecycle marker to certify it; refusing to append (@/inv/event-log)"
    )]
    UncertifiedTornTail,
    /// A custom preset failed structural validation; carries the underlying
    /// validator's forwarded message.
    #[error("invalid custom gate preset: {0}")]
    InvalidGatePreset(String),
    /// A custom-preset ancestor path is occupied by a non-directory.
    #[error("invalid custom gate preset: custom preset parent '{0}' is not a directory")]
    GatePresetParentNotDirectory(String),
    /// The canonical custom-preset target was already occupied.
    #[error("custom gate preset '{0}' already exists")]
    GatePresetAlreadyExists(String),
}

/// Canonical data-root-relative path for one gate-run result record.
///
/// A run id is exactly one path component. Rejecting empty, nested, and lexical
/// escape spellings here keeps finalization and every storage reader on the same
/// `gate-runs/<run-id>/result.json` shape.
pub fn gate_run_result_relative_path(
    run_id: &str,
) -> Result<RootRelativePath, RepositoryLayoutError> {
    let run_id_path = RootRelativePath::parse(run_id)?;
    if run_id_path.depth() != 1 {
        return Err(RepositoryLayoutError::LexicalEscape(run_id.to_owned()));
    }
    RootRelativePath::parse(format!("gate-runs/{run_id}/result.json"))
}

/// Exact canonical result leaves for ordinary direct gate-run directories in a
/// captured root listing. Non-directory clutter is intentionally ignored.
pub(crate) fn captured_gate_run_result_paths(
    image: &RepositoryImage,
) -> Result<Option<Vec<VirtualPath>>, RepositoryLayoutError> {
    let root = VirtualPath::GATE_RUNS;
    let Some(listing) = image.directory_listings().get(&root) else {
        return Ok(None);
    };
    listing
        .names()
        .iter()
        .filter_map(|run_id| {
            let child = VirtualPath::data(format!("gate-runs/{run_id}")).ok()?;
            matches!(
                image.entries().get(&child),
                Some(RepositoryEntry::Directory { .. })
            )
            .then_some(run_id)
        })
        .map(|run_id| {
            gate_run_result_relative_path(run_id).and_then(|path| VirtualPath::data(path.as_path()))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// Serialize an issue to its exact on-disk bytes (pretty, no trailing newline).
pub fn serialize_issue(issue: &Issue) -> Result<Vec<u8>, MutationError> {
    canonical_json(issue, true)
}

/// Serialize a gate-run result to its exact on-disk bytes.
pub fn serialize_gate_run(result: &GateRunResult) -> Result<Vec<u8>, MutationError> {
    canonical_json(result, true)
}

/// Serialize a fresh (empty) membership index to its exact on-disk bytes.
///
/// The finalizer is the sole authority over `index.json` serialization, so
/// repository initialization renders its empty index through the same canonical
/// serializer a later membership update uses; the two never disagree.
pub fn fresh_index_bytes() -> Result<Vec<u8>, MutationError> {
    super::RepositoryIndex::default()
        .to_pretty_bytes()
        .map_err(Into::into)
}

/// Serialize one event to its exact single-line JSONL representation (no newline).
pub fn serialize_event(event: &Event) -> Result<Vec<u8>, MutationError> {
    canonical_json(event, false)
}

/// Serialize typed repository state after recursively sorting every JSON object.
fn canonical_json<T: Serialize + ?Sized>(
    value: &T,
    pretty: bool,
) -> Result<Vec<u8>, MutationError> {
    let canonical = canonicalize_json(serde_json::to_value(value)?);
    if pretty {
        Ok(serde_json::to_vec_pretty(&canonical)?)
    } else {
        Ok(serde_json::to_vec(&canonical)?)
    }
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        Value::Object(entries) => {
            let mut entries = entries
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(entries.into_iter().collect())
        }
        scalar => scalar,
    }
}

/// Whether a captured `events.jsonl` prefix ends in a malformed, unterminated
/// (torn) tail: a non-empty prefix not ending in a newline whose final line is
/// not valid JSON. The finalizer — not command code — owns this inspection.
pub fn prefix_has_torn_tail(prefix: &[u8]) -> bool {
    if prefix.is_empty() || prefix.ends_with(b"\n") {
        return false;
    }
    let final_line = prefix
        .rsplit(|byte| *byte == b'\n')
        .next()
        .unwrap_or(prefix);
    serde_json::from_slice::<serde_json::Value>(final_line).is_err()
}

/// Compose the bytes appended at the captured log length: add one separator
/// newline only when the log ends in an unterminated `final_line`, then append each
/// event line with exactly one trailing newline.
fn compose_events(final_line: &[u8], lines: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    if !final_line.is_empty() {
        out.push(b'\n');
    }
    for line in lines {
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    out
}

/// The captured event log's length and the bytes after its last newline, from
/// a whole read or a tail that holds that whole line.
///
/// # Errors
///
/// [`CaptureError::TailOnlyCapture`] for a tail that holds only part of the
/// final line, which cannot say whether that line is torn.
fn captured_log_end(image: &RepositoryImage) -> Result<(u64, &[u8]), MutationError> {
    if let RepositoryEntry::FileTail { tail, .. } = image.entry(&VirtualPath::EVENTS)? {
        let final_line = tail
            .final_line()
            .ok_or(CaptureError::TailOnlyCapture(VirtualPath::EVENTS))?;
        return Ok((tail.length(), final_line));
    }
    let log = captured_file_bytes(image, &VirtualPath::EVENTS)?.unwrap_or_default();
    let final_line = log.rsplit(|byte| *byte == b'\n').next().unwrap_or_default();
    Ok((log.len() as u64, final_line))
}

/// Read the captured bytes of a `Data(...)` file entry, or `None` when absent.
fn captured_file_bytes<'a>(
    image: &'a RepositoryImage,
    path: &VirtualPath,
) -> Result<Option<&'a [u8]>, MutationError> {
    Ok(image.file_bytes(path)?)
}

/// Parse the captured issue at `id`, or `None` when the issue file is absent.
fn captured_issue(image: &RepositoryImage, id: &IssueId) -> Result<Option<Issue>, MutationError> {
    let path = VirtualPath::issue_record(id);
    match captured_file_bytes(image, &path)? {
        None => Ok(None),
        Some(bytes) => serde_json::from_slice(bytes).map(Some).map_err(|error| {
            MutationError::MalformedRecord {
                path: path.clone(),
                reason: error.to_string(),
            }
        }),
    }
}

/// Parse the captured event-log prefix into known events (tolerating one
/// certified torn tail, matching the read path).
fn captured_events(image: &RepositoryImage) -> Result<Vec<Event>, MutationError> {
    let path = VirtualPath::EVENTS;
    let Some(bytes) = captured_file_bytes(image, &path)? else {
        return Ok(Vec::new());
    };
    let text = std::str::from_utf8(bytes).map_err(|error| MutationError::MalformedRecord {
        path: path.clone(),
        reason: error.to_string(),
    })?;
    crate::domain::event_log::parse_known_events(text).map_err(|error| {
        MutationError::MalformedRecord {
            path: path.clone(),
            reason: error.to_string(),
        }
    })
}

/// Whether the captured event tail already reflects `agent`'s claim of `issue_id`
/// as the current assignment: the most recent assignment-affecting event for the
/// issue is an `IssueClaimed` by `agent` with no later release.
fn event_tail_reflects_claim(events: &[Event], issue_id: &IssueId, agent: &Assignee) -> bool {
    events
        .iter()
        .rev()
        .find_map(|event| match event.body() {
            EventBody::IssueClaimed {
                issue_id: id,
                assignee,
                ..
            } if id == issue_id => Some(assignee == agent),
            EventBody::IssueReleased { issue_id: id, .. } if id == issue_id => Some(false),
            _ => None,
        })
        .unwrap_or(false)
}

/// The single typed-to-byte finalizer for repository-owned records.
///
/// Given the captured `image`, the reused `context`, and closed semantic
/// `intents`, it produces one exact [`MaterializationPlan`]. When no transition occurs the plan carries an empty delta having
/// sampled neither an identifier nor the mutation timestamp, so a no-op emits no
/// bytes. Identifiers follow the frozen order — the indices of new issue ids
/// first (creation order, derived by the caller and reserved here), gate-run/record
/// ids next, event ids last after canonical event ordering — and one mutation
/// timestamp is applied to every transition that occurs.
pub fn finalize(
    layout: &RepositoryLayout,
    image: &RepositoryImage,
    context: &MutationContext,
    intents: &[MutationIntent],
) -> Result<MaterializationPlan, MutationError> {
    let delta = finalize_delta(layout, image, context, intents)?;
    Ok(MaterializationPlan::new(image, delta))
}

pub(super) fn finalize_delta(
    layout: &RepositoryLayout,
    image: &RepositoryImage,
    context: &MutationContext,
    intents: &[MutationIntent],
) -> Result<RepositoryDelta, MutationError> {
    context.begin();
    let mut actions: Vec<RepositoryAction> = Vec::new();
    let mut pending_events: Vec<PendingEvent> = Vec::new();
    let mut index_creations: Vec<IssueId> = Vec::new();
    let mut index_deletions: Vec<IssueId> = Vec::new();

    if intents.iter().any(|intent| {
        matches!(
            intent,
            MutationIntent::CreateIssue { .. } | MutationIntent::CreateIssueBatch { .. }
        )
    }) {
        let parent = VirtualPath::ISSUES;
        if matches!(
            image.entry(&parent)?,
            crate::repository_state::RepositoryEntry::Absent
        ) {
            actions.push(RepositoryAction::CreateDirectory {
                path: parent,
                owner: RepositoryActionOwner::RepositoryStateMutation,
                expected: ExpectedPreimage::Absent,
            });
        }
    }

    // Pass 1: issue creations, in canonical request order, hold the leading
    // indices their callers derived ids at.
    context.reserve(
        intents
            .iter()
            .map(|intent| match intent {
                MutationIntent::CreateIssue { .. } => 1,
                MutationIntent::CreateIssueBatch { drafts, .. } => drafts.len(),
                _ => 0,
            })
            .sum(),
    );
    for intent in intents {
        if let MutationIntent::CreateIssue { draft } = intent {
            let now = context.timestamp();
            let mut issue = (**draft).clone();
            let id = issue.id.clone();
            issue.created_at = now;
            issue.updated_at = now;
            if issue.state == State::Ready {
                issue.first_ready_at = Some(now);
            } else {
                issue.first_ready_at = None;
            }
            for gate in issue.gates_status.values_mut() {
                gate.updated_at = now;
            }
            actions.push(write_issue_action(image, &issue)?);
            index_creations.push(id.clone());
            pending_events.push(PendingEvent {
                phase: 0,
                tag: EventTag::IssueCreated,
                primary: Some(id.clone()),
                secondary: String::new(),
                ordinal: pending_events.len(),
                body: EventBody::draft_issue_created(&issue),
            });
        }
    }

    for intent in intents {
        if let MutationIntent::CreateIssueBatch {
            drafts,
            dependencies,
        } = intent
        {
            for (index, draft) in drafts.iter().enumerate() {
                let now = context.timestamp();
                let mut issue = draft.clone();
                issue.dependencies = dependencies
                    .iter()
                    .filter(|(dependent, _)| *dependent == index)
                    .map(|(_, dependency)| drafts[*dependency].id.clone())
                    .collect();
                issue.dependencies.sort();
                issue.dependencies.dedup();
                issue.state = if issue.dependencies.is_empty() {
                    State::Ready
                } else {
                    State::Backlog
                };
                issue.created_at = now;
                issue.updated_at = now;
                issue.first_ready_at = (issue.state == State::Ready).then_some(now);
                actions.push(write_issue_action(image, &issue)?);
                index_creations.push(issue.id.clone());
                pending_events.push(PendingEvent {
                    phase: 0,
                    tag: EventTag::IssueCreated,
                    primary: Some(issue.id.clone()),
                    secondary: String::new(),
                    ordinal: pending_events.len(),
                    body: EventBody::draft_issue_created(&issue),
                });
                if !issue.dependencies.is_empty() {
                    pending_events.push(PendingEvent {
                        phase: 1,
                        tag: EventTag::IssueUpdated,
                        primary: Some(issue.id.clone()),
                        secondary: String::new(),
                        ordinal: pending_events.len(),
                        body: EventBody::draft_issue_updated(
                            issue.id.clone(),
                            "batch-create".to_string(),
                            vec!["dependencies".to_string(), "state".to_string()],
                        ),
                    });
                }
            }
        }
    }

    // Gate-run records get identifiers next, in (issue id, gate key) order. They
    // are allocated before the issue updates so each gate status published with a
    // run names that run.
    let mut gate_runs: Vec<&GateRunResult> = intents
        .iter()
        .filter_map(|intent| match intent {
            MutationIntent::RecordGateRun { draft } => Some(&**draft),
            _ => None,
        })
        .collect();
    gate_runs.sort_by(|left, right| {
        (left.issue_id.as_str(), left.gate_key.as_str())
            .cmp(&(right.issue_id.as_str(), right.gate_key.as_str()))
    });
    let gate_runs: Vec<(String, &GateRunResult)> = gate_runs
        .into_iter()
        .map(|draft| (context.allocate(), draft))
        .collect();

    // Pass 2: convergent claim upserts (existing issues; no new identifiers).
    for intent in intents {
        if let MutationIntent::ClaimIssue { issue_id, agent } = intent {
            let events = captured_events(image)?;
            let current = captured_issue(image, issue_id)?
                .ok_or_else(|| MutationError::MissingIssue(issue_id.clone()))?;
            let issue_reflects = current.assignee.as_ref() == Some(agent);
            let event_reflects = event_tail_reflects_claim(&events, issue_id, agent);

            if !issue_reflects {
                let now = context.timestamp();
                let mut issue = current;
                issue.assignee = Some(agent.clone());
                if issue.claimed_at.is_none() {
                    issue.claimed_at = Some(now);
                }
                issue.updated_at = now;
                actions.push(write_issue_action(image, &issue)?);
            }
            if !event_reflects {
                pending_events.push(PendingEvent {
                    phase: 1,
                    tag: EventTag::IssueClaimed,
                    primary: Some(issue_id.clone()),
                    secondary: agent.to_string(),
                    ordinal: pending_events.len(),
                    body: EventBody::draft_issue_claimed(issue_id.clone(), agent.clone()),
                });
            }
        }
    }

    // Pass 2b: full-issue semantic updates of existing issues. No identifier is
    // allocated (the id must match a captured issue). `updated_at` is stamped from the single mutation clock;
    // lifecycle history is reconciled from the captured preimage and exact state /
    // assignment transitions are stamped from the one mutation clock. A target
    // absent from the captured image is a typed error, never an implicit create.
    for intent in intents {
        if let MutationIntent::UpdateIssue { issue } = intent {
            let preimage = captured_issue(image, &issue.id)?
                .ok_or_else(|| MutationError::MissingIssue(issue.id.clone()))?;
            let now = context.timestamp();
            let mut updated = (**issue).clone();
            updated.created_at = preimage.created_at;
            updated.first_ready_at = preimage.first_ready_at;
            updated.claimed_at = preimage.claimed_at;
            updated.done_at = preimage.done_at;
            if preimage.state != State::Ready && updated.state == State::Ready {
                updated.mark_first_ready(now);
            }
            if preimage.assignee.is_none() && updated.assignee.is_some() {
                updated.mark_claimed(now);
            }
            if preimage.state != State::Done && updated.state == State::Done {
                updated.mark_done(now);
            }
            for (key, state) in &mut updated.gates_status {
                if let Some((run_id, _)) = gate_runs
                    .iter()
                    .rev()
                    .find(|(_, run)| run.issue_id == updated.id && run.gate_key.as_str() == key)
                {
                    state.run_id = Some(run_id.clone());
                }
                let records_evidence = intents.iter().any(|intent| match intent {
                    MutationIntent::RecordEvent { event, .. } => match (&**event, state.status) {
                        (
                            EventBody::GatePassed {
                                issue_id, gate_key, ..
                            },
                            GateStatus::Passed,
                        )
                        | (
                            EventBody::GateFailed {
                                issue_id, gate_key, ..
                            },
                            GateStatus::Failed,
                        ) => issue_id == &updated.id && gate_key.as_str() == key,
                        _ => false,
                    },
                    _ => false,
                });
                if records_evidence || preimage.gates_status.get(key) != Some(state) {
                    state.updated_at = now;
                }
            }
            updated.updated_at = now;
            actions.push(write_issue_action(image, &updated)?);
        }
    }

    // Pass 2c: issue deletions remove the captured record and move membership in
    // the same delta. Absence is an error, matching the old delete command
    // contract without turning this into an idempotent or implicit operation.
    for intent in intents {
        if let MutationIntent::DeleteIssue { issue_id } = intent {
            captured_issue(image, issue_id)?
                .ok_or_else(|| MutationError::MissingIssue(issue_id.clone()))?;
            let path = VirtualPath::issue_record(issue_id);
            actions.push(RepositoryAction::DeleteFile {
                path: path.clone(),
                owner: RepositoryActionOwner::RepositoryStateMutation,
                expected: expected_of(image, &path)?,
            });
            index_deletions.push(issue_id.clone());
        }
    }

    // Authored gate declarations are serialized only by the neutral declaration
    // owner. A registry edit remains an explicit typed record mutation; the
    // command cannot smuggle arbitrary repository bytes through this finalizer.
    for intent in intents {
        if let MutationIntent::EditGateRegistry { registry } = intent {
            let path = VirtualPath::GATES;
            let bytes = crate::declarations::serialize_gate_registry(registry)?;
            let current = image.entry(&path)?;
            if !matches!(
                current,
                crate::repository_state::RepositoryEntry::File {
                    content,
                    mode: FileMode::Regular,
                } if content.bytes() == bytes.as_slice()
            ) {
                actions.push(RepositoryAction::WriteFile {
                    path,
                    owner: RepositoryActionOwner::RepositoryStateMutation,
                    expected: ExpectedPreimage::of(current),
                    bytes,
                    mode: FileMode::Regular,
                });
            }
        }
    }

    // Project-defined gate presets are repository-owned records too. Their name
    // is both the semantic identity and canonical filename stem, so publication
    // is create-only and path-safe rather than an ambient upsert.
    for intent in intents {
        if let MutationIntent::CreateGatePreset { preset } = intent {
            preset
                .validate()
                .map_err(|error| MutationError::InvalidGatePreset(error.to_string()))?;
            // Nearest-root-first, so each parent exists before its child.
            for parent in [VirtualPath::CONFIG_DIR, VirtualPath::GATE_PRESETS] {
                match image.entry(&parent)? {
                    crate::repository_state::RepositoryEntry::Absent => {
                        actions.push(RepositoryAction::CreateDirectory {
                            path: parent,
                            owner: RepositoryActionOwner::RepositoryStateMutation,
                            expected: ExpectedPreimage::Absent,
                        });
                    }
                    crate::repository_state::RepositoryEntry::Directory { .. } => {}
                    _ => {
                        return Err(MutationError::GatePresetParentNotDirectory(
                            parent.relative().as_str().to_string(),
                        ))
                    }
                }
            }
            let presets_dir = VirtualPath::GATE_PRESETS;
            let path = VirtualPath::data(format!(
                "{}/{}.json",
                presets_dir.relative().as_str(),
                preset.name
            ))?;
            if !matches!(
                image.entry(&path)?,
                crate::repository_state::RepositoryEntry::Absent
            ) {
                return Err(MutationError::GatePresetAlreadyExists(preset.name.clone()));
            }
            actions.push(RepositoryAction::WriteFile {
                path,
                owner: RepositoryActionOwner::RepositoryStateMutation,
                expected: ExpectedPreimage::Absent,
                bytes: canonical_json(&**preset, true)?,
                mode: FileMode::Regular,
            });
        }
    }

    // Pass 3: gate-run records, identified above, are published.
    if !gate_runs.is_empty() {
        let parent = VirtualPath::GATE_RUNS;
        if matches!(
            image.entry(&parent)?,
            crate::repository_state::RepositoryEntry::Absent
        ) {
            actions.push(RepositoryAction::CreateDirectory {
                path: parent,
                owner: RepositoryActionOwner::RepositoryStateMutation,
                expected: ExpectedPreimage::Absent,
            });
        }
    }
    for (run_id, draft) in gate_runs {
        let mut result = draft.clone();
        result.run_id = run_id.clone();
        if result.origin == crate::domain::GateVerdictOrigin::Attested {
            let now = context.timestamp();
            result.started_at = now;
            result.completed_at = Some(now);
        }
        let result_path = gate_run_result_relative_path(&run_id)?;
        let dir = VirtualPath::data(result_path.as_path().parent().ok_or_else(|| {
            RepositoryLayoutError::LexicalEscape(result_path.as_path().display().to_string())
        })?)?;
        let file = VirtualPath::data(result_path.as_path())?;
        // The run directory is freshly named by an allocated id, so it is absent.
        actions.push(RepositoryAction::CreateDirectory {
            path: dir,
            owner: RepositoryActionOwner::RepositoryStateMutation,
            expected: ExpectedPreimage::Absent,
        });
        actions.push(RepositoryAction::WriteFile {
            path: file,
            owner: RepositoryActionOwner::RepositoryStateMutation,
            expected: ExpectedPreimage::Absent,
            bytes: serialize_gate_run(&result)?,
            mode: FileMode::Regular,
        });
    }

    // Explicit provenance/audit events join the pending set.
    for intent in intents {
        if let MutationIntent::RecordEvent { phase, event } = intent {
            let (primary, secondary) = event_identities(event);
            pending_events.push(PendingEvent {
                phase: *phase,
                tag: event.tag(),
                primary,
                secondary,
                ordinal: pending_events.len(),
                body: (**event).clone(),
            });
        }
    }

    // Pass 4: order events canonically, assign their identifiers last, stamp the
    // single mutation timestamp, and compose the exact audit append.
    if let Some(action) = compose_event_action(image, context, pending_events)? {
        actions.push(action);
    }

    // Index membership: creations extend `all_ids` and clear any `deleted_ids`
    // entry, derived from the captured index preimage in the same delta.
    if !index_creations.is_empty() || !index_deletions.is_empty() {
        actions.push(index_membership_action(
            image,
            &index_creations,
            &index_deletions,
        )?);
    }

    RepositoryDelta::new(layout, actions).map_err(Into::into)
}

/// Ordering phase of the record a dispatch annotation contributes.
///
/// The record annotates the whole append rather than one transition within it, so it
/// occupies the last phase: it is composed after every event it accompanies, without
/// displacing any of them and without depending on tag comparison against an existing
/// phase.
const ANNOTATION_PHASE: u8 = u8::MAX;

/// The audit record this context's invocation-scoped annotation contributes to one
/// event pass, or `None` when dispatch installed no annotation.
///
/// Every finalizer path that appends events composes them through
/// [`compose_event_action`], so a path cannot publish its events while omitting the
/// record. Each plan that emits events carries the record, which is what makes the
/// record and the mutation it explains durable in the same application.
fn dispatch_annotation_event(context: &MutationContext, ordinal: usize) -> Option<PendingEvent> {
    let body = match context.dispatch_annotation()? {
        MutationContextAnnotation::LinkedCheckoutWriteOverridden {
            checkout,
            declared_stance,
            invocation_override,
        } => EventBody::draft_linked_checkout_write_overridden(
            checkout.clone(),
            *declared_stance,
            *invocation_override,
        ),
    };
    let (primary, secondary) = event_identities(&body);
    Some(PendingEvent {
        phase: ANNOTATION_PHASE,
        tag: body.tag(),
        primary,
        secondary,
        ordinal,
        body,
    })
}

/// Compose the audit-log append action for a set of pending events over the
/// captured `events.jsonl` length: admit the record the context's dispatch annotation
/// contributes, order them canonically, assign identifiers (via the context, in the
/// caller's frozen allocation order), stamp the single mutation timestamp, certify a
/// torn tail (a profile lifecycle marker is required, and the certifier is composed
/// immediately after the torn partial), and produce the appended bytes. Returns `None`
/// when there are no events. This is the event pass of [`finalize`], factored out so
/// [`finalize_audit_append`] can reuse the one torn-tail authority — and so the
/// annotation record reaches both paths from one place.
fn compose_event_action(
    image: &RepositoryImage,
    context: &MutationContext,
    mut pending_events: Vec<PendingEvent>,
) -> Result<Option<RepositoryAction>, MutationError> {
    if pending_events.is_empty() {
        return Ok(None);
    }
    // The annotation record accompanies real events only: the emptiness check above
    // precedes it, so a no-transition plan that emits no bytes publishes no record
    // either. Admitting it here — before ordering, identity assignment, and byte
    // composition — puts it in the same append, and therefore the same plan, as the
    // events it accompanies.
    if let Some(record) = dispatch_annotation_event(context, pending_events.len()) {
        pending_events.push(record);
    }
    let (length, final_line) = captured_log_end(image)?;
    // The finalizer owns torn-tail evidence. The reader (parse_known_events) accepts
    // a malformed, unterminated partial line only when the line IMMEDIATELY following
    // it is a profile lifecycle `{ isolated_torn_tail: true }` certifier. Detect this
    // before sampling time or allocating identifiers.
    let torn_tail = prefix_has_torn_tail(final_line);
    let has_marker = pending_events
        .iter()
        .any(|pending| matches!(pending.body, EventBody::ProfileLifecycle { .. }));
    if torn_tail && !has_marker {
        // Appending non-certifying events after an uncertified torn tail would
        // produce a log the reader rejects; refuse instead of corrupting it.
        return Err(MutationError::UncertifiedTornTail);
    }

    let now = context.timestamp();
    // Assign identifiers in canonical event order, stamping the single mutation
    // timestamp and the torn-tail evidence.
    pending_events.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
    let mut events = pending_events
        .into_iter()
        .map(|pending| {
            Event::new(
                context.allocate(),
                now,
                pending.body.with_isolated_torn_tail(torn_tail),
            )
        })
        .collect::<Vec<_>>();
    // Over a torn tail, the certifying marker must be composed as the line
    // immediately following the torn partial; the remaining events keep their
    // canonical order after it.
    let certifier = torn_tail
        .then(|| {
            events
                .iter()
                .position(|event| matches!(event.body(), EventBody::ProfileLifecycle { .. }))
        })
        .flatten();
    if let Some(marker) = certifier {
        events[..=marker].rotate_right(1);
    }
    let lines = events
        .iter()
        .map(serialize_event)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(RepositoryAction::AppendEvents {
        owner: RepositoryActionOwner::RepositoryStateMutation,
        expected: expected_of(image, &VirtualPath::EVENTS)?,
        offset: length,
        bytes: compose_events(final_line, &lines),
    }))
}

/// Compose the exact `events.jsonl` append action for repository-owned provenance
/// events emitted by a delta the record finalizer does not otherwise build — the
/// init/profile finalizers, whose deltas also carry asset/registry/record file
/// actions. Each `(phase, body)` pair is a drafted event body; this resets the context's frozen allocation order, then
/// assigns identifiers and the single mutation timestamp and certifies a torn tail
/// exactly as [`finalize`] does. Returns `None` when `events` is empty (no append,
/// no identity or time sampling).
///
/// The context must be the operation's single reused [`MutationContext`] and must
/// not also drive a full [`finalize`] in the same operation (both reset the frozen
/// order); the init/profile paths use only this entry.
pub(crate) fn finalize_audit_append(
    image: &RepositoryImage,
    context: &MutationContext,
    events: Vec<(u8, EventBody)>,
) -> Result<Option<RepositoryAction>, MutationError> {
    if events.is_empty() {
        return Ok(None);
    }
    context.begin();
    let pending = events
        .into_iter()
        .enumerate()
        .map(|(ordinal, (phase, body))| {
            let (primary, secondary) = event_identities(&body);
            PendingEvent {
                phase,
                tag: body.tag(),
                primary,
                secondary,
                ordinal,
                body,
            }
        })
        .collect();
    compose_event_action(image, context, pending)
}

/// Build the write action for an issue, using the captured preimage so a create
/// expects absence and an upsert expects the exact captured file.
fn write_issue_action(
    image: &RepositoryImage,
    issue: &Issue,
) -> Result<RepositoryAction, MutationError> {
    let path = VirtualPath::issue_record(&issue.id);
    Ok(RepositoryAction::WriteFile {
        path: path.clone(),
        owner: RepositoryActionOwner::RepositoryStateMutation,
        expected: expected_of(image, &path)?,
        bytes: serialize_issue(issue)?,
        mode: FileMode::Regular,
    })
}

/// Derive the exact expected preimage for `path` from the captured image.
fn expected_of(
    image: &RepositoryImage,
    path: &VirtualPath,
) -> Result<ExpectedPreimage, MutationError> {
    Ok(ExpectedPreimage::of(image.entry(path)?))
}

/// Derive the deterministic membership index write from the captured preimage.
fn index_membership_action(
    image: &RepositoryImage,
    created_ids: &[IssueId],
    deleted_ids: &[IssueId],
) -> Result<RepositoryAction, MutationError> {
    let path = VirtualPath::INDEX;
    let expected = expected_of(image, &path)?;
    let mut index = match captured_file_bytes(image, &path)? {
        Some(bytes) => super::RepositoryIndex::parse(bytes).map_err(|error| {
            MutationError::MalformedRecord {
                path: path.clone(),
                reason: error.to_string(),
            }
        })?,
        None => super::RepositoryIndex::default(),
    };
    for id in created_ids {
        if !index.all_ids.contains(id) {
            index.all_ids.push(id.clone());
        }
        index.deleted_ids.retain(|deleted| deleted != id);
    }
    for id in deleted_ids {
        index.all_ids.retain(|active| active != id);
        if !index.deleted_ids.contains(id) {
            index.deleted_ids.push(id.clone());
        }
    }
    Ok(RepositoryAction::WriteFile {
        path,
        owner: RepositoryActionOwner::RepositoryStateMutation,
        expected,
        bytes: index.to_pretty_bytes()?,
        mode: FileMode::Regular,
    })
}

/// Primary/secondary ordering identities for an event.
fn event_identities(body: &EventBody) -> (Option<IssueId>, String) {
    let issue_id = body.issue_id().cloned();
    match body {
        EventBody::GatePassed { gate_key, .. }
        | EventBody::GateFailed { gate_key, .. }
        | EventBody::GateAdded { gate_key, .. }
        | EventBody::GateRemoved { gate_key, .. }
        | EventBody::GateDefinitionCreated { gate_key, .. }
        | EventBody::GateDefinitionUpdated { gate_key, .. }
        | EventBody::GateDefinitionRemoved { gate_key, .. } => (issue_id, gate_key.to_string()),
        EventBody::IssueClaimed { assignee, .. } => (issue_id, assignee.to_string()),
        _ => (issue_id, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{GateState, GateStatus};
    use crate::repository_state::{
        CaptureBudget, CaptureSpec, CapturedBytes, RepositoryEntry, RepositoryImage,
        RepositoryIndex, RepositoryRootEvidence, SUPPORTED_INDEX_SCHEMA_VERSION,
    };
    use std::collections::{BTreeMap, HashMap};

    fn layout() -> RepositoryLayout {
        RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "wt", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap()
    }

    fn fixed_instant() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-19T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn ctx() -> MutationContext {
        MutationContext::deterministic([7u8; 32], fixed_instant())
    }

    struct PanicMutationClock;

    impl MutationClock for PanicMutationClock {
        fn now(&self) -> DateTime<Utc> {
            panic!("a no-op must not sample the mutation clock");
        }
    }

    fn budget() -> CaptureBudget {
        CaptureBudget {
            max_listings: 2,
            max_bytes: 1 << 20,
            max_depth: 6,
        }
    }

    #[test]
    fn test_gate_run_result_relative_path_accepts_exactly_one_run_id_component() {
        assert_eq!(
            gate_run_result_relative_path("run-123").unwrap().as_path(),
            std::path::Path::new("gate-runs/run-123/result.json")
        );
        for invalid in ["", ".", "nested/run", "../run", "run\\child"] {
            assert!(
                gate_run_result_relative_path(invalid).is_err(),
                "{invalid:?} must not escape the one-child gate-run layout"
            );
        }
    }

    /// Build an image over a fixed set of `Data(...)` entries.
    fn image_with(entries: Vec<(VirtualPath, RepositoryEntry)>) -> RepositoryImage {
        let paths: Vec<VirtualPath> = entries.iter().map(|(path, _)| path.clone()).collect();
        let spec = CaptureSpec::new(paths, budget()).unwrap();
        RepositoryImage::close(
            layout(),
            spec,
            entries.into_iter().collect::<BTreeMap<_, _>>(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    }

    fn file_entry(bytes: &[u8]) -> RepositoryEntry {
        RepositoryEntry::File {
            content: CapturedBytes::new("obj", bytes.to_vec()).unwrap(),
            mode: FileMode::Regular,
        }
    }

    fn seeded_issue(id: &IssueId, assignee: Option<Assignee>) -> Issue {
        let mut issue = Issue::draft(id.clone(), "Title".into(), "Body".into());
        issue.assignee = assignee;
        issue.created_at = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        issue.updated_at = issue.created_at;
        issue
    }

    /// The id the creation tests' drafts carry.
    fn created_id() -> IssueId {
        crate::test_utils::issue_id(0x1111_4111_8111_1111_1111_1111_1111_1111)
    }

    fn map_order_draft(reverse: bool) -> Issue {
        let mut issue = Issue::draft(created_id(), "Canonical".into(), "Body".into());
        let context_entries = [("alpha", "one"), ("zeta", "two")];
        let gate_entries = [
            (
                "alpha-gate",
                GateState {
                    status: GateStatus::Passed,
                    updated_by: Some("agent:alpha".parse().unwrap()),
                    updated_at: fixed_instant(),
                    run_id: None,
                },
            ),
            (
                "zeta-gate",
                GateState {
                    status: GateStatus::Failed,
                    updated_by: Some("agent:zeta".parse().unwrap()),
                    updated_at: fixed_instant(),
                    run_id: None,
                },
            ),
        ];
        let order = if reverse { [1, 0] } else { [0, 1] };
        issue.context = order
            .iter()
            .map(|index| {
                let (key, value) = context_entries[*index];
                (key.to_string(), value.to_string())
            })
            .collect::<HashMap<_, _>>();
        issue.gates_status = order
            .iter()
            .map(|index| gate_entries[*index].clone())
            .map(|(key, value)| (key.to_string(), value))
            .collect::<HashMap<_, _>>();
        issue
    }

    #[test]
    fn test_finalizer_canonicalizes_issue_maps_for_retry_bytes_and_hash() {
        let first_draft = map_order_draft(false);
        let retry_draft = map_order_draft(true);
        assert_eq!(first_draft, retry_draft);
        assert_eq!(
            serialize_issue(&first_draft).unwrap(),
            serialize_issue(&retry_draft).unwrap(),
            "issue bytes must not depend on HashMap insertion order"
        );

        let index_bytes = serde_json::to_vec_pretty(&RepositoryIndex {
            schema_version: 2,
            all_ids: Vec::new(),
            deleted_ids: Vec::new(),
        })
        .unwrap();
        let created_id = created_id();
        let image = image_with(vec![
            (
                VirtualPath::data("issues").unwrap(),
                RepositoryEntry::Absent,
            ),
            (VirtualPath::INDEX, file_entry(&index_bytes)),
            (VirtualPath::EVENTS, RepositoryEntry::Absent),
            (
                VirtualPath::issue_record(&created_id),
                RepositoryEntry::Absent,
            ),
        ]);
        let first = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::CreateIssue {
                draft: Box::new(first_draft),
            }],
        )
        .unwrap();
        let retry = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::CreateIssue {
                draft: Box::new(retry_draft),
            }],
        )
        .unwrap();

        assert_eq!(
            serde_json::to_vec(first.delta()).unwrap(),
            serde_json::to_vec(retry.delta()).unwrap(),
            "delta bytes must be stable across reconstructed values"
        );
    }

    /// The override fact a dispatch site installs when it lets an invocation
    /// mutate the store of a linked checkout the declared stance refuses.
    fn override_annotation() -> MutationContextAnnotation {
        MutationContextAnnotation::LinkedCheckoutWriteOverridden {
            checkout: std::path::PathBuf::from("/repo/.worktrees/feature"),
            declared_stance: LinkedCheckoutWriteStance::Refuse,
            invocation_override: LinkedCheckoutWriteStance::Allow,
        }
    }

    /// The deterministic context of [`ctx`] carrying that override fact, as the
    /// production factory stamps it into every context it builds.
    fn annotated_ctx() -> MutationContext {
        MutationContext::with_annotation(
            IdAuthority::from_seed([7u8; 32]),
            Box::new(FixedMutationClock::new(fixed_instant())),
            Some(override_annotation()),
        )
    }

    /// The image a single issue creation finalizes over.
    fn creation_image() -> RepositoryImage {
        let index_bytes = serde_json::to_vec_pretty(&RepositoryIndex {
            schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
            all_ids: Vec::new(),
            deleted_ids: Vec::new(),
        })
        .unwrap();
        let created_id = created_id();
        image_with(vec![
            (
                VirtualPath::data("issues").unwrap(),
                RepositoryEntry::Absent,
            ),
            (VirtualPath::INDEX, file_entry(&index_bytes)),
            (VirtualPath::EVENTS, RepositoryEntry::Absent),
            (
                VirtualPath::issue_record(&created_id),
                RepositoryEntry::Absent,
            ),
        ])
    }

    /// The exact bytes a delta appends to the event log.
    fn appended_event_bytes(delta: &RepositoryDelta) -> Option<Vec<u8>> {
        delta.appended_event_bytes().map(<[u8]>::to_vec)
    }

    /// The events a delta appends, as the event-log reader recovers them.
    fn appended_events(delta: &RepositoryDelta) -> Vec<Event> {
        appended_event_bytes(delta)
            .map(|bytes| {
                crate::domain::event_log::parse_known_events(std::str::from_utf8(&bytes).unwrap())
                    .unwrap()
            })
            .unwrap_or_default()
    }

    /// The override records among `events`.
    fn override_records(events: &[Event]) -> Vec<&Event> {
        events
            .iter()
            .filter(|event| {
                matches!(
                    event.body(),
                    EventBody::LinkedCheckoutWriteOverridden { .. }
                )
            })
            .collect()
    }

    #[test]
    fn test_finalize_publishes_the_dispatch_override_record_in_the_mutating_plan() {
        let image = creation_image();
        let intents = [MutationIntent::CreateIssue {
            draft: Box::new(map_order_draft(false)),
        }];
        let plan = finalize(&layout(), &image, &annotated_ctx(), &intents).unwrap();

        let created_id = created_id();
        assert!(
            plan.delta()
                .actions()
                .iter()
                .any(|action| action.path() == &VirtualPath::issue_record(&created_id)),
            "the record the override permitted is part of the same finalized plan"
        );

        let events = appended_events(plan.delta());
        let records = override_records(&events);
        assert_eq!(
            records.len(),
            1,
            "the permitted mutation carries exactly one override record; events: {events:?}"
        );
        let EventBody::LinkedCheckoutWriteOverridden {
            checkout,
            declared_stance,
            invocation_override,
            ..
        } = records[0].body()
        else {
            unreachable!("the record was selected by its variant");
        };
        let MutationContextAnnotation::LinkedCheckoutWriteOverridden {
            checkout: annotated_checkout,
            declared_stance: annotated_declared,
            invocation_override: annotated_override,
        } = override_annotation();
        assert_eq!(
            (checkout, declared_stance, invocation_override),
            (
                &annotated_checkout,
                &annotated_declared,
                &annotated_override
            ),
            "the record names the checkout and the two stances the dispatch site set"
        );
        assert!(
            !records[0].id().is_empty(),
            "the finalizer assigns the record an identity like any other event"
        );

        let creation_timestamp = events
            .iter()
            .find_map(|event| match event.body() {
                EventBody::IssueCreated { .. } => Some(event.timestamp()),
                _ => None,
            })
            .expect("the mutation's own event is in the same append");
        assert_eq!(
            creation_timestamp,
            records[0].timestamp(),
            "the record carries the one mutation timestamp, so it belongs to this mutation"
        );
        let record_position = events
            .iter()
            .position(|event| {
                matches!(
                    event.body(),
                    EventBody::LinkedCheckoutWriteOverridden { .. }
                )
            })
            .expect("the record is in the append");
        assert_eq!(
            record_position,
            events.len() - 1,
            "the record annotates the whole append, so canonical order places it last; events: {events:?}"
        );
    }

    #[test]
    fn test_finalize_without_dispatch_override_publishes_no_record_and_no_extra_event() {
        let image = creation_image();
        let intents = [MutationIntent::CreateIssue {
            draft: Box::new(map_order_draft(false)),
        }];
        let unannotated = appended_event_bytes(
            finalize(&layout(), &image, &ctx(), &intents)
                .unwrap()
                .delta(),
        )
        .expect("an issue creation appends its own events");
        let annotated = appended_event_bytes(
            finalize(&layout(), &image, &annotated_ctx(), &intents)
                .unwrap()
                .delta(),
        )
        .expect("an issue creation appends its own events");

        assert!(
            override_records(
                &crate::domain::event_log::parse_known_events(
                    std::str::from_utf8(&unannotated).unwrap()
                )
                .unwrap()
            )
            .is_empty(),
            "an invocation with no override in effect emits no override record"
        );
        assert!(
            annotated.starts_with(&unannotated),
            "the override record is purely additive: every byte the unannotated mutation \
             writes is written unchanged, and the record follows them"
        );
    }

    #[test]
    fn test_finalize_noop_mutation_under_dispatch_override_emits_no_bytes() {
        // The override is in effect, but the mutation transitions nothing. The
        // documented no-transition property holds: no delta, no identity, no time,
        // and therefore no override record either.
        let agent: Assignee = "agent:worker-1".parse().unwrap();
        let issue = seeded_issue(
            &crate::test_utils::issue_id(0x5555_4555_8555_5555_5555_5555_5555_5555),
            Some(agent.clone()),
        );
        let issue_bytes = serialize_issue(&issue).unwrap();
        let event = crate::test_utils::stamped_event(
            EventBody::draft_issue_claimed(issue.id.clone(), agent.clone()),
            "e",
            fixed_instant(),
        );
        let events_bytes = compose_events(&[], &[serialize_event(&event).unwrap()]);
        let image = image_with(vec![
            (
                VirtualPath::issue_record(&issue.id),
                file_entry(&issue_bytes),
            ),
            (VirtualPath::EVENTS, file_entry(&events_bytes)),
        ]);
        let context = MutationContext::with_annotation(
            IdAuthority::random(),
            Box::new(PanicMutationClock),
            Some(override_annotation()),
        );
        let plan = finalize(
            &layout(),
            &image,
            &context,
            &[MutationIntent::ClaimIssue {
                issue_id: issue.id.clone(),
                agent,
            }],
        )
        .unwrap();
        assert!(
            plan.delta().actions().is_empty(),
            "an override in effect during a no-op mutation emits no bytes"
        );
        assert!(
            context.ids.sampled_seed().is_none(),
            "a no-op under an override must not draw the production random seed"
        );
    }

    /// An image holding the event log as `tail`.
    fn tail_image(tail: crate::repository_state::EventLogTail) -> RepositoryImage {
        let mut spec = CaptureSpec::new([], budget()).unwrap();
        spec.discover_tail(VirtualPath::EVENTS).unwrap();
        let mode = FileMode::Regular;
        let entries = BTreeMap::from([(
            VirtualPath::EVENTS,
            RepositoryEntry::FileTail { tail, mode },
        )]);
        RepositoryImage::close(layout(), spec, entries, BTreeMap::new(), BTreeMap::new()).unwrap()
    }

    /// An append over a log read as its tail lands at the log's length and
    /// expects that tail, with no whole-log digest behind it.
    #[test]
    fn test_event_append_over_a_tail_read_offsets_at_its_length_and_expects_the_tail() {
        let log = b"{}\n".repeat(2 * crate::repository_state::EVENT_LOG_TAIL_WINDOW as usize);
        let tail = crate::repository_state::EventLogTail::of_bytes(&log);
        let image = tail_image(tail.clone());

        let action = finalize_audit_append(&image, &ctx(), vec![(2, profile_lifecycle_marker())])
            .unwrap()
            .expect("a non-empty audit append composes an action");

        let RepositoryAction::AppendEvents {
            offset, expected, ..
        } = action
        else {
            unreachable!("an audit append appends to the event log");
        };
        assert_eq!(offset, log.len() as u64);
        assert_eq!(
            expected,
            ExpectedPreimage::FileTail {
                tail,
                mode: FileMode::Regular
            }
        );
    }

    /// A tail whose window starts inside the final line cannot say whether that
    /// line is torn, so the composer refuses it rather than guess.
    #[test]
    fn test_event_append_refuses_a_tail_that_does_not_hold_the_final_line() {
        let window = crate::repository_state::EVENT_LOG_TAIL_WINDOW;
        let partial =
            crate::repository_state::EventLogTail::new(2 * window, vec![b'x'; window as usize]);
        let image = tail_image(partial.unwrap());

        assert!(matches!(
            finalize_audit_append(&image, &ctx(), vec![(2, profile_lifecycle_marker())]),
            Err(MutationError::Capture(CaptureError::TailOnlyCapture(_)))
        ));
    }

    #[test]
    fn test_finalize_audit_append_publishes_the_dispatch_override_record_with_its_events() {
        // The second finalizer path composes its events through the same event pass,
        // so the dispatch fact reaches it without a second edit.
        let image = image_with(vec![(VirtualPath::EVENTS, RepositoryEntry::Absent)]);
        let action = finalize_audit_append(
            &image,
            &annotated_ctx(),
            vec![(2, profile_lifecycle_marker())],
        )
        .unwrap()
        .expect("a non-empty audit append composes an action");
        let RepositoryAction::AppendEvents { bytes, .. } = &action else {
            unreachable!("an audit append appends to the event log");
        };
        let events =
            crate::domain::event_log::parse_known_events(std::str::from_utf8(bytes).unwrap())
                .unwrap();
        assert_eq!(
            override_records(&events).len(),
            1,
            "the initialization-style append carries the override record too; events: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event.body(), EventBody::ProfileLifecycle { .. })),
            "the caller's own event is in the same append; events: {events:?}"
        );

        assert!(
            finalize_audit_append(&image, &annotated_ctx(), Vec::new())
                .unwrap()
                .is_none(),
            "an override in effect over an empty audit append still emits nothing"
        );
    }

    #[test]
    fn test_create_issue_stamps_all_gate_evidence_with_operation_time() {
        let mut draft = map_order_draft(false);
        let preserved = draft
            .gates_status
            .iter()
            .map(|(key, gate)| (key.clone(), (gate.status, gate.updated_by.clone())))
            .collect::<HashMap<_, _>>();
        let index_bytes = serde_json::to_vec_pretty(&RepositoryIndex {
            schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
            all_ids: Vec::new(),
            deleted_ids: Vec::new(),
        })
        .unwrap();
        let created_id = created_id();
        let image = image_with(vec![
            (
                VirtualPath::data("issues").unwrap(),
                RepositoryEntry::Absent,
            ),
            (VirtualPath::INDEX, file_entry(&index_bytes)),
            (VirtualPath::EVENTS, RepositoryEntry::Absent),
            (
                VirtualPath::issue_record(&created_id),
                RepositoryEntry::Absent,
            ),
        ]);

        // Deliberately prove the finalizer owns these values rather than accepting
        // caller timestamps embedded in the draft.
        for gate in draft.gates_status.values_mut() {
            gate.updated_at = DateTime::parse_from_rfc3339("2001-02-03T04:05:06Z")
                .unwrap()
                .with_timezone(&Utc);
        }
        let plan = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::CreateIssue {
                draft: Box::new(draft),
            }],
        )
        .unwrap();
        let created: Issue = plan
            .delta()
            .actions()
            .iter()
            .find_map(|action| match action {
                RepositoryAction::WriteFile { path, bytes, .. }
                    if path == &VirtualPath::issue_record(&created_id) =>
                {
                    serde_json::from_slice(bytes).ok()
                }
                _ => None,
            })
            .unwrap();

        assert_eq!(created.gates_status.len(), 2);
        for (key, gate) in created.gates_status {
            assert_eq!(gate.updated_at, fixed_instant());
            assert_eq!(
                (gate.status, gate.updated_by),
                preserved.get(&key).cloned().unwrap()
            );
        }
    }

    #[test]
    fn test_canonical_serialization_preserves_repository_record_shapes() {
        let issue_bytes = serialize_issue(&map_order_draft(false)).unwrap();
        assert!(issue_bytes.starts_with(b"{\n"));
        assert!(!issue_bytes.ends_with(b"\n"));

        let gate_run = GateRunResult::attestation(
            &crate::test_utils::issue_id(7),
            &"cargo-ci".parse().unwrap(),
            crate::declarations::GateStage::Postcheck,
            GateStatus::Passed,
            None,
        );
        let gate_run_bytes = serialize_gate_run(&gate_run).unwrap();
        assert!(gate_run_bytes.starts_with(b"{\n"));
        assert!(!gate_run_bytes.ends_with(b"\n"));
        assert_eq!(
            serde_json::from_slice::<GateRunResult>(&gate_run_bytes).unwrap(),
            gate_run,
            "gate-run bytes preserve the repository record"
        );

        let event = crate::test_utils::stamped_event(
            EventBody::draft_gate_definition_created("cargo-ci".parse().unwrap()),
            "event-1",
            fixed_instant(),
        );
        let event_bytes = serialize_event(&event).unwrap();
        assert!(event_bytes.starts_with(b"{"));
        assert!(!event_bytes.contains(&b'\n'));
        assert!(!event_bytes.ends_with(b"\n"));
        let round_trip: Event = serde_json::from_slice(&event_bytes).unwrap();
        assert_eq!(round_trip, event);
    }

    /// Every repository log line with a current tag re-serializes to its own
    /// bytes; the only lines the parser skips carry retired tags.
    #[test]
    fn test_serialize_event_reproduces_current_lines_and_parser_skips_only_retired_tags() {
        let log = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.jit/events.jsonl"),
        )
        .unwrap();
        let reproduced = log
            .lines()
            .map(|line| {
                let events = crate::domain::event_log::parse_known_events(line).unwrap();
                if events.is_empty() {
                    let tag = serde_json::from_str::<Value>(line).unwrap()["type"].clone();
                    assert!(
                        ["document_archived", "lifecycle_timestamps_backfilled"]
                            .iter()
                            .any(|retired| tag == *retired),
                        "parser skipped a line with a current or unknown tag: {line}"
                    );
                }
                events.iter().for_each(|event| {
                    assert_eq!(
                        std::str::from_utf8(&serialize_event(event).unwrap()).unwrap(),
                        line
                    );
                });
                usize::from(!events.is_empty())
            })
            .sum::<usize>();
        assert!(reproduced > 0, "the repository log holds current events");
    }

    /// Every repository issue record, current-format gate-run result and the
    /// membership index re-serialize to their stored bytes; every other gate-run
    /// result decodes and reproduces its ids.
    #[test]
    fn test_repository_records_decode_and_reproduce_their_stored_issue_ids() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.jit");
        let stored = |path: &std::path::Path| -> Value {
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
        };
        let records = |directory: &str| {
            std::fs::read_dir(data.join(directory))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>()
        };
        let issues = records("issues")
            .into_iter()
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .map(|path| {
                let raw = stored(&path);
                let issue: Issue = serde_json::from_value(raw.clone()).unwrap();
                assert_eq!(
                    serialize_issue(&issue).unwrap(),
                    canonical_json(&raw, true).unwrap(),
                    "{}",
                    path.display()
                );
            })
            .count();
        let current_format_runs = records("gate-runs")
            .into_iter()
            .filter(|directory| {
                let path = directory.join("result.json");
                let bytes = std::fs::read(&path).unwrap();
                let run: GateRunResult = serde_json::from_slice(&bytes).unwrap();
                let written = serialize_gate_run(&run).unwrap();
                let raw = stored(&path);
                let decoded: Value = serde_json::from_slice(&written).unwrap();
                ["issue_id", "run_id", "gate_key"].iter().for_each(|field| {
                    assert_eq!(decoded[field], raw[field], "{} {field}", path.display());
                });
                let keys = |value: &Value| {
                    value.as_object().map(|object| {
                        object
                            .keys()
                            .cloned()
                            .collect::<std::collections::BTreeSet<_>>()
                    })
                };
                let is_current_format = keys(&decoded) == keys(&raw);
                if is_current_format {
                    assert_eq!(written, bytes, "{}", path.display());
                }
                is_current_format
            })
            .count();
        let index_path = data.join("index.json");
        let index_bytes = std::fs::read(&index_path).unwrap();
        let index: RepositoryIndex = serde_json::from_slice(&index_bytes).unwrap();

        assert_eq!(index.to_pretty_bytes().unwrap(), index_bytes);
        assert!(
            issues > 0 && current_format_runs > 0,
            "the repository holds issue records and current-format gate runs"
        );
    }

    #[test]
    fn test_update_issue_stamps_changed_gate_state_from_mutation_context() {
        let id = &crate::test_utils::issue_id(0x1111_4111_8111_1111_1111_1111_1111_1111);
        let old_time = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut preimage = seeded_issue(id, None);
        preimage.gates_status.insert(
            "tests".into(),
            GateState {
                status: GateStatus::Pending,
                updated_by: None,
                updated_at: old_time,
                run_id: None,
            },
        );
        let mut updated = preimage.clone();
        updated.gates_status.insert(
            "tests".into(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("agent:reviewer".parse().unwrap()),
                // Checker/manual callers submit no lifecycle timestamp.
                updated_at: DateTime::default(),
                run_id: None,
            },
        );
        let bytes = serialize_issue(&preimage).unwrap();
        let image = image_with(vec![(VirtualPath::issue_record(id), file_entry(&bytes))]);
        let intent = MutationIntent::UpdateIssue {
            issue: Box::new(updated),
        };
        let first = finalize(&layout(), &image, &ctx(), std::slice::from_ref(&intent)).unwrap();
        let retry = finalize(&layout(), &image, &ctx(), &[intent]).unwrap();
        assert_eq!(first.delta(), retry.delta());
        let written = first
            .delta()
            .actions()
            .iter()
            .find_map(|action| match action {
                RepositoryAction::WriteFile { path, bytes, .. }
                    if path == &VirtualPath::issue_record(id) =>
                {
                    Some(bytes)
                }
                _ => None,
            })
            .unwrap();
        let persisted: Issue = serde_json::from_slice(written).unwrap();
        assert_eq!(persisted.gates_status["tests"].updated_at, fixed_instant());
        assert_eq!(persisted.updated_at, fixed_instant());
    }

    #[test]
    fn test_update_issue_gate_state_names_the_run_recorded_with_it() {
        let id = &crate::test_utils::issue_id(0x1111_4111_8111_1111_1111_1111_1111_1111);
        let preimage = seeded_issue(id, None);
        let mut updated = preimage.clone();
        updated.gates_status.insert(
            "review".into(),
            GateState {
                status: GateStatus::Passed,
                updated_by: None,
                updated_at: DateTime::default(),
                run_id: None,
            },
        );
        let bytes = serialize_issue(&preimage).unwrap();
        let image = image_with(vec![
            (VirtualPath::issue_record(id), file_entry(&bytes)),
            (VirtualPath::GATE_RUNS, RepositoryEntry::Absent),
        ]);
        let intents = [
            MutationIntent::UpdateIssue {
                issue: Box::new(updated),
            },
            MutationIntent::RecordGateRun {
                draft: Box::new(GateRunResult::attestation(
                    id,
                    &"review".parse().unwrap(),
                    crate::declarations::GateStage::Postcheck,
                    GateStatus::Passed,
                    None,
                )),
            },
        ];
        let plan = finalize(&layout(), &image, &ctx(), &intents).unwrap();
        let written = |wanted: &dyn Fn(&VirtualPath) -> bool| {
            plan.delta()
                .actions()
                .iter()
                .find_map(|action| match action {
                    RepositoryAction::WriteFile { path, bytes, .. } if wanted(path) => {
                        Some(bytes.clone())
                    }
                    _ => None,
                })
                .unwrap()
        };
        let issue_path = VirtualPath::issue_record(id);
        let persisted: Issue =
            serde_json::from_slice(&written(&|path| path == &issue_path)).unwrap();
        let run: GateRunResult = serde_json::from_slice(&written(&|path| {
            path.relative().as_str().starts_with("gate-runs/")
        }))
        .unwrap();
        assert_eq!(persisted.gates_status["review"].run_id, Some(run.run_id));
    }

    #[test]
    fn test_id_authority_is_deterministic_and_ordered() {
        let ids = IdAuthority::from_seed([1u8; 32]);
        let a = ids.uuid_at(0);
        let b = ids.uuid_at(1);
        assert_ne!(a, b);
        assert_eq!(a, IdAuthority::from_seed([1u8; 32]).uuid_at(0));
        // A distinct seed yields a distinct id at the same index.
        assert_ne!(a, IdAuthority::from_seed([2u8; 32]).uuid_at(0));
        // The derived value is a parseable UUID.
        assert!(uuid::Uuid::parse_str(&a).is_ok());
    }

    #[test]
    fn test_random_authority_draws_seed_once_lazily() {
        // A production authority draws nothing until the first allocation, then
        // reuses that one seed. Combined with `sample_random_seed` drawing exactly
        // one `Uuid::new_v4`, this establishes the one-source-sample contract.
        let ids = IdAuthority::random();
        assert!(
            ids.sampled_seed().is_none(),
            "an unused production authority draws no seed"
        );
        let first = ids.uuid_at(0);
        let seed = ids
            .sampled_seed()
            .expect("the first allocation draws the seed");
        let second = ids.uuid_at(1);
        assert_ne!(first, second);
        assert_eq!(
            ids.sampled_seed(),
            Some(seed),
            "the seed is drawn once, then reused for every later allocation"
        );
    }

    #[test]
    fn test_claim_noop_when_issue_and_event_reflect_claim_samples_nothing() {
        let agent: Assignee = "agent:worker-1".parse().unwrap();
        let issue = seeded_issue(
            &crate::test_utils::issue_id(0x1111_4111_8111_1111_1111_1111_1111_1111),
            Some(agent.clone()),
        );
        let issue_bytes = serialize_issue(&issue).unwrap();
        let event = crate::test_utils::stamped_event(
            EventBody::draft_issue_claimed(issue.id.clone(), agent.clone()),
            "e",
            fixed_instant(),
        );
        let events_bytes = compose_events(&[], &[serialize_event(&event).unwrap()]);
        let image = image_with(vec![
            (
                VirtualPath::issue_record(&issue.id),
                file_entry(&issue_bytes),
            ),
            (VirtualPath::EVENTS, file_entry(&events_bytes)),
        ]);
        // The shared clock proves the no-op samples no time.
        let context = MutationContext::new(
            IdAuthority::from_seed([9u8; 32]),
            Box::new(PanicMutationClock),
        );
        let delta = finalize(
            &layout(),
            &image,
            &context,
            &[MutationIntent::ClaimIssue {
                issue_id: issue.id.clone(),
                agent,
            }],
        )
        .unwrap();
        assert!(
            delta.delta().actions().is_empty(),
            "fully reflected claim is a no-op"
        );
    }

    #[test]
    fn test_noop_claim_production_authority_draws_no_id_and_no_time() {
        // The strongest no-op guarantee: with a real production random authority
        // and a clock that panics on use, a fully-reflected claim acquire draws
        // neither the random source (the seed stays undrawn) nor the clock.
        let agent: Assignee = "agent:worker-1".parse().unwrap();
        let issue = seeded_issue(
            &crate::test_utils::issue_id(0x4444_4444_8444_4444_4444_4444_4444_4444),
            Some(agent.clone()),
        );
        let issue_bytes = serialize_issue(&issue).unwrap();
        let event = crate::test_utils::stamped_event(
            EventBody::draft_issue_claimed(issue.id.clone(), agent.clone()),
            "e",
            fixed_instant(),
        );
        let events_bytes = compose_events(&[], &[serialize_event(&event).unwrap()]);
        let image = image_with(vec![
            (
                VirtualPath::issue_record(&issue.id),
                file_entry(&issue_bytes),
            ),
            (VirtualPath::EVENTS, file_entry(&events_bytes)),
        ]);
        let context = MutationContext::new(IdAuthority::random(), Box::new(PanicMutationClock));
        let plan = finalize(
            &layout(),
            &image,
            &context,
            &[MutationIntent::ClaimIssue {
                issue_id: issue.id.clone(),
                agent,
            }],
        )
        .unwrap();
        assert!(
            plan.delta().actions().is_empty(),
            "fully reflected claim emits no bytes"
        );
        assert!(
            context.ids.sampled_seed().is_none(),
            "a no-op acquire must not draw the production random seed"
        );
    }

    #[test]
    fn test_claim_missing_event_only_emits_event() {
        let agent: Assignee = "agent:worker-1".parse().unwrap();
        // Issue already assigned but the event log is empty (crash after save).
        let issue = seeded_issue(
            &crate::test_utils::issue_id(0x2222_4222_8222_2222_2222_2222_2222_2222),
            Some(agent.clone()),
        );
        let issue_bytes = serialize_issue(&issue).unwrap();
        let image = image_with(vec![
            (
                VirtualPath::issue_record(&issue.id),
                file_entry(&issue_bytes),
            ),
            (VirtualPath::EVENTS, RepositoryEntry::Absent),
        ]);
        let delta = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::ClaimIssue {
                issue_id: issue.id.clone(),
                agent,
            }],
        )
        .unwrap();
        // Exactly one action: the missing events.jsonl append. No issue rewrite.
        assert_eq!(delta.delta().actions().len(), 1);
        let action = &delta.delta().actions()[0];
        assert_eq!(action.path(), &VirtualPath::EVENTS);
    }

    #[test]
    fn test_claim_fresh_emits_issue_and_event() {
        let agent: Assignee = "agent:worker-1".parse().unwrap();
        let issue = seeded_issue(
            &crate::test_utils::issue_id(0x3333_4333_8333_3333_3333_3333_3333_3333),
            None,
        );
        let issue_bytes = serialize_issue(&issue).unwrap();
        let image = image_with(vec![
            (
                VirtualPath::issue_record(&issue.id),
                file_entry(&issue_bytes),
            ),
            (VirtualPath::EVENTS, RepositoryEntry::Absent),
        ]);
        let intent = MutationIntent::ClaimIssue {
            issue_id: issue.id.clone(),
            agent: agent.clone(),
        };
        let context = ctx();
        let first = finalize(&layout(), &image, &context, std::slice::from_ref(&intent)).unwrap();
        let retry = finalize(&layout(), &image, &context, &[intent]).unwrap();
        assert_eq!(
            serde_json::to_vec(first.delta()).unwrap(),
            serde_json::to_vec(retry.delta()).unwrap(),
            "a captured-image retry preserves the complete claim delta bytes"
        );
        // Two actions: the assigned issue and the appended claim event.
        assert_eq!(first.delta().actions().len(), 2);
        // The issue write carries the assignee and a stamped claimed_at.
        let issue_action = first
            .delta()
            .actions()
            .iter()
            .find(|action| action.path() == &VirtualPath::issue_record(&issue.id))
            .unwrap();
        let RepositoryAction::WriteFile { bytes, .. } = issue_action else {
            panic!("expected issue write");
        };
        let mut expected_issue = issue;
        expected_issue.assignee = Some(agent.clone());
        expected_issue.claimed_at = Some(fixed_instant());
        expected_issue.updated_at = fixed_instant();
        assert_eq!(bytes, &serialize_issue(&expected_issue).unwrap());

        let expected_event = crate::test_utils::stamped_event(
            EventBody::draft_issue_claimed(expected_issue.id, agent),
            IdAuthority::from_seed([7u8; 32]).uuid_at(0),
            fixed_instant(),
        );
        let mut expected_event_line = serialize_event(&expected_event).unwrap();
        expected_event_line.push(b'\n');
        assert_eq!(
            first.delta().appended_event_bytes(),
            Some(expected_event_line.as_slice())
        );
    }

    #[test]
    fn test_finalize_empty_intents_samples_neither_time_nor_identity() {
        let context = MutationContext::new(IdAuthority::random(), Box::new(PanicMutationClock));
        let plan = finalize(&layout(), &image_with(Vec::new()), &context, &[]).unwrap();

        assert!(plan.delta().actions().is_empty());
        assert!(
            context.ids.sampled_seed().is_none(),
            "empty intents must not draw the production identifier seed"
        );
    }

    #[test]
    fn test_finalize_record_event_reuses_identity_time_and_bytes_on_retry() {
        let image = image_with(vec![(VirtualPath::EVENTS, RepositoryEntry::Absent)]);
        let intent = MutationIntent::RecordEvent {
            phase: 0,
            event: Box::new(EventBody::draft_gate_definition_created(
                "cargo-ci".parse().unwrap(),
            )),
        };
        let context = ctx();
        let first = finalize(&layout(), &image, &context, std::slice::from_ref(&intent)).unwrap();
        let retry = finalize(&layout(), &image, &context, &[intent]).unwrap();

        assert_eq!(
            serde_json::to_vec(first.delta()).unwrap(),
            serde_json::to_vec(retry.delta()).unwrap(),
            "the same image and context produce byte-identical provenance plans"
        );
        let expected_event = crate::test_utils::stamped_event(
            EventBody::draft_gate_definition_created("cargo-ci".parse().unwrap()),
            IdAuthority::from_seed([7u8; 32]).uuid_at(0),
            fixed_instant(),
        );
        let mut expected_line = serialize_event(&expected_event).unwrap();
        expected_line.push(b'\n');
        assert_eq!(
            first.delta().appended_event_bytes(),
            Some(expected_line.as_slice())
        );
    }

    #[test]
    fn test_finalize_gate_run_assigns_identity_and_serializes_attestation() {
        let run_id = IdAuthority::from_seed([7u8; 32]).uuid_at(0);
        let image = image_with(vec![
            (VirtualPath::GATE_RUNS, RepositoryEntry::Absent),
            (VirtualPath::EVENTS, RepositoryEntry::Absent),
        ]);
        let draft = GateRunResult::attestation(
            &crate::test_utils::issue_id(7),
            &"cargo-ci".parse().unwrap(),
            crate::declarations::GateStage::Postcheck,
            GateStatus::Passed,
            None,
        );
        let intent = MutationIntent::RecordGateRun {
            draft: Box::new(draft.clone()),
        };
        let provenance = MutationIntent::RecordEvent {
            phase: 5,
            event: Box::new(EventBody::draft_gate_definition_created(
                "cargo-ci".parse().unwrap(),
            )),
        };
        let intents = [intent, provenance];
        let context = ctx();
        let first = finalize(&layout(), &image, &context, &intents).unwrap();
        let retry = finalize(&layout(), &image, &context, &intents).unwrap();

        assert_eq!(
            serde_json::to_vec(first.delta()).unwrap(),
            serde_json::to_vec(retry.delta()).unwrap(),
            "a retry keeps the assigned run record bytes"
        );
        let result_path = gate_run_result_relative_path(&run_id).unwrap();
        let result_path = VirtualPath::data(result_path.as_path()).unwrap();
        let bytes = first
            .delta()
            .actions()
            .iter()
            .find_map(|action| match action {
                RepositoryAction::WriteFile { path, bytes, .. } if path == &result_path => {
                    Some(bytes)
                }
                _ => None,
            })
            .expect("the allocated gate-run record is written");
        let mut expected = draft;
        expected.run_id = run_id;
        expected.started_at = fixed_instant();
        expected.completed_at = Some(fixed_instant());
        assert_eq!(bytes, &serialize_gate_run(&expected).unwrap());

        let expected_event = crate::test_utils::stamped_event(
            EventBody::draft_gate_definition_created("cargo-ci".parse().unwrap()),
            IdAuthority::from_seed([7u8; 32]).uuid_at(1),
            fixed_instant(),
        );
        let mut expected_line = serialize_event(&expected_event).unwrap();
        expected_line.push(b'\n');
        assert_eq!(
            first.delta().appended_event_bytes(),
            Some(expected_line.as_slice()),
            "provenance follows the gate-run allocation and carries its next identity"
        );
    }

    #[test]
    fn test_finalize_creation_orders_events_and_assigns_frozen_identity_sequence() {
        let ids = IdAuthority::from_seed([7u8; 32]);
        let mut draft = Issue::draft(created_id(), "New".into(), "Body".into());
        draft.state = State::Ready;
        let intents = [
            MutationIntent::CreateIssue {
                draft: Box::new(draft),
            },
            MutationIntent::RecordEvent {
                phase: 5,
                event: Box::new(EventBody::draft_gate_definition_created(
                    "bbb".parse().unwrap(),
                )),
            },
            MutationIntent::RecordEvent {
                phase: 5,
                event: Box::new(EventBody::draft_gate_definition_created(
                    "aaa".parse().unwrap(),
                )),
            },
        ];
        let image = creation_image();
        let context = ctx();
        let first = finalize(&layout(), &image, &context, &intents).unwrap();
        let retry = finalize(&layout(), &image, &context, &intents).unwrap();
        assert_eq!(
            serde_json::to_vec(first.delta()).unwrap(),
            serde_json::to_vec(retry.delta()).unwrap(),
            "retry keeps issue, membership, and event bytes identical"
        );
        let events = appended_events(first.delta());

        assert!(matches!(
            events.iter().map(Event::body).collect::<Vec<_>>().as_slice(),
            [
                EventBody::IssueCreated { .. },
                EventBody::GateDefinitionCreated { gate_key: first_key, .. },
                EventBody::GateDefinitionCreated { gate_key: second_key, .. },
            ] if events[0].id() == ids.uuid_at(1)
                && events[0].timestamp() == fixed_instant()
                && events[1].id() == ids.uuid_at(2)
                && events[1].timestamp() == fixed_instant()
                && first_key.as_str() == "aaa"
                && events[2].id() == ids.uuid_at(3)
                && events[2].timestamp() == fixed_instant()
                && second_key.as_str() == "bbb"
        ), "events are canonically ordered before the finalizer allocates their identities: {events:?}");
    }

    #[test]
    fn test_issue_id_at_derives_the_frozen_creation_sequence_for_the_seed() {
        let ids = IdAuthority::from_seed([7u8; 32]);
        let context = ctx();
        let derived = (0..3).map(|index| context.issue_id_at(index));
        let repeated = (0..3).map(|index| context.issue_id_at(index));

        assert!(derived.zip(repeated).zip(0..3).all(
            |((first, second), index)| first == second && first.as_str() == ids.uuid_at(index)
        ));
    }

    #[test]
    fn test_finalize_batch_creation_keeps_draft_ids_and_reserves_their_indices() {
        let drafts = [
            crate::test_utils::issue_id(61),
            crate::test_utils::issue_id(62),
        ]
        .map(|id| Issue::draft(id, "Batch".into(), "Body".into()))
        .to_vec();
        let draft_ids = drafts
            .iter()
            .map(|draft| draft.id.clone())
            .collect::<Vec<_>>();
        let index_bytes = serde_json::to_vec_pretty(&RepositoryIndex {
            schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
            all_ids: Vec::new(),
            deleted_ids: Vec::new(),
        })
        .unwrap();
        let image = image_with(
            [
                (
                    VirtualPath::data("issues").unwrap(),
                    RepositoryEntry::Absent,
                ),
                (VirtualPath::INDEX, file_entry(&index_bytes)),
                (VirtualPath::EVENTS, RepositoryEntry::Absent),
            ]
            .into_iter()
            .chain(
                draft_ids
                    .iter()
                    .map(|id| (VirtualPath::issue_record(id), RepositoryEntry::Absent)),
            )
            .collect(),
        );
        let plan = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::CreateIssueBatch {
                drafts,
                dependencies: vec![(1, 0)],
            }],
        )
        .unwrap();

        let written = draft_ids
            .iter()
            .map(|id| {
                plan.delta()
                    .actions()
                    .iter()
                    .find_map(|action| match action {
                        RepositoryAction::WriteFile { path, bytes, .. }
                            if path == &VirtualPath::issue_record(id) =>
                        {
                            serde_json::from_slice::<Issue>(bytes).ok()
                        }
                        _ => None,
                    })
                    .expect("each draft is written at its own id")
            })
            .collect::<Vec<_>>();
        assert_eq!(written[1].dependencies, [draft_ids[0].clone()]);
        let events = appended_events(plan.delta());
        let created = events
            .iter()
            .filter_map(|event| match event.body() {
                EventBody::IssueCreated { issue_id, .. } => Some(issue_id.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(created, draft_ids, "creation events record the drafts' ids");
        let ids = IdAuthority::from_seed([7u8; 32]);
        assert!(
            events
                .iter()
                .zip(2..)
                .all(|(event, index)| event.id() == ids.uuid_at(index)),
            "event ids follow the two reserved creation indices: {events:?}"
        );
    }

    #[test]
    fn test_create_issue_stamps_lifecycle_and_membership() {
        let index_bytes = serde_json::to_vec_pretty(&RepositoryIndex {
            schema_version: 2,
            all_ids: Vec::new(),
            deleted_ids: Vec::new(),
        })
        .unwrap();
        let draft = {
            let mut issue = Issue::draft(created_id(), "New".into(), "Body".into());
            issue.state = State::Ready;
            issue
        };
        let image = image_with(vec![
            (
                VirtualPath::data("issues").unwrap(),
                RepositoryEntry::Absent,
            ),
            (VirtualPath::INDEX, file_entry(&index_bytes)),
            (VirtualPath::EVENTS, RepositoryEntry::Absent),
            // The draft carries its id; seed that id's absent slot.
            (
                VirtualPath::issue_record(&created_id()),
                RepositoryEntry::Absent,
            ),
        ]);
        let delta = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::CreateIssue {
                draft: Box::new(draft),
            }],
        )
        .unwrap();
        let expected_id = created_id();
        let issue_action = delta
            .delta()
            .actions()
            .iter()
            .find(|action| action.path() == &VirtualPath::issue_record(&expected_id))
            .expect("issue write present");
        let RepositoryAction::WriteFile { bytes, .. } = issue_action else {
            panic!("expected issue write");
        };
        let written: Issue = serde_json::from_slice(bytes).unwrap();
        assert_eq!(written.id, expected_id);
        assert_eq!(written.created_at, fixed_instant());
        assert_eq!(written.first_ready_at, Some(fixed_instant()));
        // Membership index carries the new id.
        let index_action = delta
            .delta()
            .actions()
            .iter()
            .find(|action| action.path() == &VirtualPath::INDEX)
            .expect("index write present");
        let RepositoryAction::WriteFile { bytes, .. } = index_action else {
            panic!("expected index write");
        };
        let index: RepositoryIndex = serde_json::from_slice(bytes).unwrap();
        assert!(index.all_ids.contains(&expected_id));
    }

    #[test]
    fn test_delete_issue_removes_record_and_moves_index_membership() {
        let issue = seeded_issue(
            &crate::test_utils::issue_id(0x1111_4111_8111_1111_1111_1111_1111_1111),
            None,
        );
        let issue_bytes = serialize_issue(&issue).unwrap();
        let index_bytes = canonical_json(
            &RepositoryIndex {
                schema_version: 2,
                all_ids: vec![issue.id.clone(), crate::test_utils::issue_id(49)],
                deleted_ids: Vec::new(),
            },
            true,
        )
        .unwrap();
        let image = image_with(vec![
            (
                VirtualPath::issue_record(&issue.id),
                file_entry(&issue_bytes),
            ),
            (VirtualPath::INDEX, file_entry(&index_bytes)),
        ]);

        let plan = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::DeleteIssue {
                issue_id: issue.id.clone(),
            }],
        )
        .unwrap();

        assert!(plan.delta().actions().iter().any(|action| matches!(
            action,
            RepositoryAction::DeleteFile { path, .. } if path == &VirtualPath::issue_record(&issue.id)
        )));
        let index_action = plan
            .delta()
            .actions()
            .iter()
            .find(|action| action.path() == &VirtualPath::INDEX)
            .expect("index write present");
        let RepositoryAction::WriteFile { bytes, .. } = index_action else {
            panic!("expected index write");
        };
        let index: RepositoryIndex = serde_json::from_slice(bytes).unwrap();
        assert_eq!(index.all_ids, [crate::test_utils::issue_id(49)]);
        assert_eq!(index.deleted_ids, [issue.id]);
    }

    #[test]
    fn test_create_issue_rejects_future_index_without_a_plan() {
        let draft = Issue::draft(created_id(), "New".into(), "Body".into());
        let created_id = created_id();
        let index_bytes = serde_json::to_vec(&serde_json::json!({
            "schema_version": SUPPORTED_INDEX_SCHEMA_VERSION + 1,
            "all_ids": [],
            "deleted_ids": []
        }))
        .unwrap();
        let image = image_with(vec![
            (
                VirtualPath::data("issues").unwrap(),
                RepositoryEntry::Absent,
            ),
            (VirtualPath::INDEX, file_entry(&index_bytes)),
            (VirtualPath::EVENTS, RepositoryEntry::Absent),
            (
                VirtualPath::issue_record(&created_id),
                RepositoryEntry::Absent,
            ),
        ]);

        let error = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::CreateIssue {
                draft: Box::new(draft),
            }],
        )
        .expect_err("a future index must prevent plan construction");
        assert!(error
            .to_string()
            .contains("newer than this binary supports"));
    }

    #[test]
    fn test_delete_issue_rejects_duplicate_deleted_index_without_a_plan() {
        let issue = seeded_issue(
            &crate::test_utils::issue_id(0x1111_4111_8111_1111_1111_1111_1111_1111),
            None,
        );
        let issue_bytes = serialize_issue(&issue).unwrap();
        let index_bytes = serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "all_ids": [issue.id],
            "deleted_ids": [crate::test_utils::issue_id(1), crate::test_utils::issue_id(1)]
        }))
        .unwrap();
        let image = image_with(vec![
            (
                VirtualPath::issue_record(&issue.id),
                file_entry(&issue_bytes),
            ),
            (VirtualPath::INDEX, file_entry(&index_bytes)),
        ]);

        let error = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::DeleteIssue {
                issue_id: issue.id.clone(),
            }],
        )
        .expect_err("duplicate deleted membership must prevent plan construction");
        assert!(error.to_string().contains("duplicate deleted issue id"));
    }

    #[test]
    fn test_update_issue_stamps_transition_lifecycle_from_one_mutation_time() {
        let issue = seeded_issue(
            &crate::test_utils::issue_id(0x2222_4222_8222_2222_2222_2222_2222_2222),
            None,
        );
        let created_at = issue.created_at;
        let mut updated = issue.clone();
        updated.state = State::Done;
        updated.created_at = DateTime::UNIX_EPOCH;
        updated.done_at = Some(DateTime::UNIX_EPOCH);
        let bytes = serialize_issue(&issue).unwrap();
        let image = image_with(vec![(
            VirtualPath::issue_record(&issue.id),
            file_entry(&bytes),
        )]);

        let plan = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::UpdateIssue {
                issue: Box::new(updated),
            }],
        )
        .unwrap();
        let action = plan
            .delta()
            .actions()
            .iter()
            .find(|action| action.path() == &VirtualPath::issue_record(&issue.id))
            .unwrap();
        let RepositoryAction::WriteFile { bytes, .. } = action else {
            panic!("expected issue write");
        };
        let persisted: Issue = serde_json::from_slice(bytes).unwrap();
        assert_eq!(persisted.created_at, created_at);
        assert_eq!(persisted.updated_at, fixed_instant());
        assert_eq!(persisted.done_at, Some(fixed_instant()));
    }

    #[test]
    fn test_torn_tail_detection() {
        assert!(!prefix_has_torn_tail(b""));
        assert!(!prefix_has_torn_tail(b"{\"type\":\"x\"}\n"));
        assert!(prefix_has_torn_tail(b"{\"type\":\"x\"}\n{\"partial"));
        assert!(!prefix_has_torn_tail(b"{\"a\":1}"));
    }

    /// The finalizer's event-log action over `image`: its offset and bytes.
    fn event_append(image: &RepositoryImage, intents: &[MutationIntent]) -> (u64, Vec<u8>) {
        let plan = finalize(&layout(), image, &ctx(), intents).unwrap();
        plan.delta()
            .actions()
            .iter()
            .find_map(|action| match action {
                RepositoryAction::AppendEvents { offset, bytes, .. } => {
                    Some((*offset, bytes.clone()))
                }
                _ => None,
            })
            .expect("the mutation appends its events")
    }

    #[test]
    fn test_finalize_appends_at_the_captured_log_length_reading_as_the_whole_log_composition() {
        let prior = serialize_event(&crate::test_utils::stamped_event(
            EventBody::draft_gate_definition_created("prior".parse().unwrap()),
            "e0",
            fixed_instant(),
        ))
        .unwrap();
        let terminated = [prior.as_slice(), b"\n"].concat();
        // A valid final line longer than the tail window, read back to its start.
        let long = serialize_event(&crate::test_utils::stamped_event(
            EventBody::draft_gate_definition_created(
                "x".repeat(2 * crate::repository_state::EVENT_LOG_TAIL_WINDOW as usize)
                    .parse()
                    .unwrap(),
            ),
            "e1",
            fixed_instant(),
        ))
        .unwrap();
        let long_log = [terminated.as_slice(), &long].concat();
        let long_image = tail_image(crate::repository_state::EventLogTail::of_bytes(&long_log));
        let record = MutationIntent::RecordEvent {
            phase: 0,
            event: Box::new(EventBody::draft_gate_definition_created(
                "cargo-ci".parse().unwrap(),
            )),
        };
        let whole = |log: Vec<u8>| {
            let image = image_with(vec![(VirtualPath::EVENTS, file_entry(&log))]);
            (log, image)
        };
        let cases = [Vec::new(), terminated, prior.clone()].map(whole);
        for (log, image) in cases.into_iter().chain([(long_log, long_image)]) {
            let (offset, appended) = event_append(&image, std::slice::from_ref(&record));

            assert_eq!(
                offset,
                log.len() as u64,
                "the offset is the captured length"
            );
            let unterminated = !log.is_empty() && !log.ends_with(b"\n");
            assert_eq!(
                appended.first() == Some(&b'\n'),
                unterminated,
                "exactly an unterminated final line earns a leading separator"
            );
            let whole = [log.as_slice(), &appended].concat();
            let read =
                crate::domain::event_log::parse_known_events(std::str::from_utf8(&whole).unwrap())
                    .unwrap();
            let prior_events =
                crate::domain::event_log::parse_known_events(std::str::from_utf8(&log).unwrap())
                    .unwrap();
            let new_events = crate::domain::event_log::parse_known_events(
                std::str::from_utf8(&appended).unwrap(),
            )
            .unwrap();
            assert_eq!(
                read,
                [prior_events, new_events].concat(),
                "the log read after the append is the captured events then the appended ones"
            );
        }
    }

    fn profile_lifecycle_marker() -> EventBody {
        EventBody::draft_profile_lifecycle(
            crate::domain::ProfileLifecycleOperation::Apply,
            vec![crate::domain::ProfileLifecycleProfile {
                id: "example"
                    .try_into()
                    .expect("the profile marker has a canonical identifier"),
                version: "1.0.0".to_string(),
                status: crate::domain::ProfileLifecycleStatus::Installed,
                variables: Default::default(),
            }],
        )
    }

    fn torn_log() -> Vec<u8> {
        let event = crate::test_utils::stamped_event(
            EventBody::draft_issue_completed(crate::test_utils::issue_id(1)),
            "e0",
            fixed_instant(),
        );
        [serialize_event(&event).unwrap().as_slice(), b"\n{\"parti"].concat()
    }

    /// A captured events.jsonl whose final line is a malformed, unterminated tail.
    fn torn_tail_image() -> RepositoryImage {
        image_with(vec![(VirtualPath::EVENTS, file_entry(&torn_log()))])
    }

    #[test]
    fn test_finalize_rejects_markerless_append_over_torn_tail() {
        // A batch with no profile lifecycle certifier appended after an uncertified
        // torn tail would produce a log the reader rejects, so finalize refuses and
        // composes nothing, also when the torn line is longer than the tail window.
        let window = crate::repository_state::EVENT_LOG_TAIL_WINDOW as usize;
        let long_torn = [torn_log().as_slice(), &vec![b'x'; 2 * window]].concat();
        let long_image = tail_image(crate::repository_state::EventLogTail::of_bytes(&long_torn));
        for image in [torn_tail_image(), long_image] {
            let event = EventBody::draft_gate_definition_created("cargo-ci".parse().unwrap());
            let result = finalize(
                &layout(),
                &image,
                &ctx(),
                &[MutationIntent::RecordEvent {
                    phase: 0,
                    event: Box::new(event),
                }],
            );
            assert!(matches!(result, Err(MutationError::UncertifiedTornTail)));
        }
    }

    #[test]
    fn test_finalize_certifier_over_torn_tail_is_the_first_appended_line() {
        let (offset, appended) = event_append(
            &torn_tail_image(),
            &[MutationIntent::RecordEvent {
                phase: 0,
                event: Box::new(profile_lifecycle_marker()),
            }],
        );

        assert_eq!(offset, torn_log().len() as u64);
        let first_line = std::str::from_utf8(&appended)
            .unwrap()
            .strip_prefix('\n')
            .and_then(|text| text.lines().next())
            .expect("a separator ends the torn partial, then the certifier follows");
        assert!(matches!(
            serde_json::from_str::<Event>(first_line).unwrap().body(),
            EventBody::ProfileLifecycle {
                isolated_torn_tail: true,
                ..
            }
        ));
        let whole = [torn_log().as_slice(), &appended].concat();
        assert!(
            crate::domain::event_log::parse_known_events(std::str::from_utf8(&whole).unwrap())
                .is_ok(),
            "the reader accepts the certified log"
        );
    }

    #[test]
    fn test_event_identities_repository_scoped_event_sorts_before_issue_scoped() {
        let (repository_scoped, _) = event_identities(&EventBody::draft_gate_definition_created(
            "g".parse().unwrap(),
        ));
        let (issue_scoped, _) = event_identities(&EventBody::draft_issue_deleted(
            crate::test_utils::issue_id(0x4000_8000_0000_0000_0001_0f0e_0d0c),
        ));

        assert!(repository_scoped < issue_scoped);
    }
}
