//! Typed mutation identity, time authority, and the sole typed-to-byte finalizer.
//!
//! Commands submit timestamp-free semantic [`MutationIntent`]s; a created issue
//! and a gate-run draft carry the id their caller derived from the context, and
//! every other identifier is the finalizer's.
//! A [`MutationContext`] — created exactly once per command operation, before
//! its conflict-retry loop — supplies deterministic identifiers (from a single seed drawn
//! once on the first allocation) and one mutation timestamp (sampled once, on the
//! first transition), so retries reuse identical record identities and times while
//! a no-op mutation samples neither identity nor time. The
//! finalizer serializes every repository-owned record (issue upserts/deletes,
//! `index.json` tombstones, gate-run/audit artifacts, and events) into one exact
//! [`RepositoryDelta`] published through one redo record, preserving
//! `@/inv/event-log` and `@/inv/atomic-writes`.
//!
//! This module performs no filesystem I/O and imports only `domain` value types
//! and the pure `repository_state` vocabulary.

use super::{
    CaptureError, DeltaError, ExpectedPreimage, FileMode, MaterializationPlan, RepositoryAction,
    RepositoryActionOwner, RepositoryDelta, RepositoryEntry, RepositoryImage, RepositoryLayout,
    RepositoryLayoutError, VirtualPath,
};
use crate::declarations::GateRegistry;
use crate::domain::event_catalog::EventTag;
use crate::domain::identity::{GateKey, GateRunId, IssueId};
use crate::domain::{
    Assignee, Event, EventBody, GateRunResult, GateStatus, GateVerdictRecord, Issue,
    LinkedCheckoutWriteStance, State,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
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

    /// Derive the id of the gate run recorded at position `index` of a mutation
    /// that creates no issue. The finalizer reserves those leading indices, so a
    /// draft built with this id keeps every later identifier in the frozen order.
    pub(crate) fn gate_run_id_at(&self, index: u64) -> GateRunId {
        GateRunId::from_uuid(self.ids.uuid_value_at(index))
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

    /// Skip the `count` indices the mutation's issue creations or gate-run
    /// drafts derived their ids at.
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
/// except a created issue's and a gate run's, which the caller derives through
/// [`MutationContext::issue_id_at`] and [`MutationContext::gate_run_id_at`];
/// callers never stamp time. The set is
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
    /// Assign `issue_id` to `agent` and append `IssueClaimed`, or do nothing when
    /// the captured issue is already assigned to `agent`.
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
    /// removes the issue record and records its tombstone in the same delta. Audit events remain explicit intents so the
    /// caller can compose deletion with dependent-edge repairs atomically.
    DeleteIssue {
        /// Canonical full issue id whose captured record must exist.
        issue_id: IssueId,
    },
    /// Record one gate-run artifact. The finalizer preserves external checker
    /// `started_at`/`completed_at` evidence verbatim; an attestation, which ran
    /// no checker, is stamped with the mutation timestamp.
    RecordGateRun {
        /// Semantic gate-run result carrying its pre-derived `run_id`.
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
    secondary: EventSecondaryIdentity,
    ordinal: usize,
    body: EventBody,
}

enum EventSecondaryIdentity {
    None,
    Gate(GateKey),
    Assignee(Assignee),
}

impl EventSecondaryIdentity {
    fn sort_key(&self) -> (Option<&GateKey>, Option<String>) {
        match self {
            Self::None => (None, None),
            Self::Gate(key) => (Some(key), None),
            Self::Assignee(assignee) => (None, Some(assignee.to_string())),
        }
    }
}

impl PendingEvent {
    fn compare(&self, other: &Self) -> std::cmp::Ordering {
        self.phase
            .cmp(&other.phase)
            .then_with(|| self.tag.as_str().cmp(other.tag.as_str()))
            .then_with(|| self.primary.cmp(&other.primary))
            .then_with(|| self.secondary.sort_key().cmp(&other.secondary.sort_key()))
            .then_with(|| self.ordinal.cmp(&other.ordinal))
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

/// The issue directories a captured gate-run root listing names that the image
/// holds as directories. `verdicts/` and other names that are not issue ids are
/// ignored.
pub(crate) fn captured_gate_run_issue_ids(image: &RepositoryImage) -> Option<Vec<IssueId>> {
    let listing = image.directory_listings().get(&VirtualPath::GATE_RUNS)?;
    Some(
        listing
            .names()
            .iter()
            .filter_map(|name| IssueId::try_from(name.as_str()).ok())
            .filter(|issue| {
                matches!(
                    image
                        .entries()
                        .get(&VirtualPath::gate_run_issue_directory(issue)),
                    Some(RepositoryEntry::Directory { .. })
                )
            })
            .collect(),
    )
}

/// Each `(issue, run)` a captured listing of a
/// [`captured_gate_run_issue_ids`] directory names as `<run-id>.json`. Other
/// names are clutter and ignored.
pub(crate) fn captured_gate_run_ids(image: &RepositoryImage) -> Vec<(IssueId, GateRunId)> {
    captured_gate_run_issue_ids(image)
        .unwrap_or_default()
        .into_iter()
        .flat_map(|issue| {
            image
                .directory_listings()
                .get(&VirtualPath::gate_run_issue_directory(&issue))
                .map(|listing| {
                    listing
                        .names()
                        .iter()
                        .filter_map(|name| name.strip_suffix(".json"))
                        .filter_map(|stem| GateRunId::try_from(stem).ok())
                        .map(|run| (issue.clone(), run))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect()
}

/// The gate key of each directory a captured `gate-runs/verdicts` listing names.
/// Names that are not gate keys, and occupants not captured as directories, are
/// ignored.
pub(crate) fn captured_gate_verdict_keys(image: &RepositoryImage) -> Vec<GateKey> {
    image
        .directory_listings()
        .get(&VirtualPath::GATE_VERDICTS)
        .map(|listing| {
            listing
                .names()
                .iter()
                .filter_map(|name| GateKey::try_from(name.as_str()).ok())
                .filter(|key| {
                    matches!(
                        image
                            .entries()
                            .get(&VirtualPath::gate_verdict_directory(key)),
                        Some(RepositoryEntry::Directory { .. })
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The path of each verdict record the captured listings of
/// [`captured_gate_verdict_keys`]' directories name. Names not of the form
/// `<inputs-digest>.json` are ignored.
pub(crate) fn captured_gate_verdict_records(image: &RepositoryImage) -> Vec<VirtualPath> {
    captured_gate_verdict_keys(image)
        .into_iter()
        .flat_map(|key| {
            image
                .directory_listings()
                .get(&VirtualPath::gate_verdict_directory(&key))
                .into_iter()
                .flat_map(|listing| listing.names().iter())
                .filter_map(|name| {
                    let digest = name.strip_suffix(".json")?;
                    crate::domain::repository_inputs::InputsDigest::try_from(digest).ok()
                })
                .map(|digest| VirtualPath::gate_verdict_record(&key, &digest))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Serialize an issue to its exact on-disk bytes (pretty, no trailing newline).
pub fn serialize_issue(issue: &Issue) -> Result<Vec<u8>, MutationError> {
    Ok(canonical_json(issue, true)?)
}

/// Serialize a gate-run result to its exact on-disk bytes.
pub fn serialize_gate_run(result: &GateRunResult) -> Result<Vec<u8>, MutationError> {
    Ok(canonical_json(result, true)?)
}

/// Serialize a gate verdict record to its exact on-disk bytes.
pub fn serialize_gate_verdict(record: &GateVerdictRecord) -> Result<Vec<u8>, MutationError> {
    Ok(canonical_json(record, true)?)
}

/// Serialize a fresh (empty) index to its exact on-disk bytes.
///
/// The finalizer is the sole authority over `index.json` serialization, so
/// repository initialization renders its empty index through the same canonical
/// serializer a later tombstone write uses; the two never disagree.
pub fn fresh_index_bytes() -> Result<Vec<u8>, MutationError> {
    super::RepositoryIndex::default()
        .to_pretty_bytes()
        .map_err(Into::into)
}

/// Serialize one event to its exact single-line JSONL representation (no newline).
pub fn serialize_event(event: &Event) -> Result<Vec<u8>, MutationError> {
    Ok(canonical_json(event, false)?)
}

/// Serialize typed repository state with every JSON object's keys sorted.
///
/// `to_value` does the sorting: `serde_json::Map` is a sorted map while the
/// `preserve_order` feature is off, as its owning test pins.
pub(crate) fn canonical_json<T: Serialize + ?Sized>(
    value: &T,
    pretty: bool,
) -> Result<Vec<u8>, serde_json::Error> {
    let canonical = serde_json::to_value(value)?;
    if pretty {
        serde_json::to_vec_pretty(&canonical)
    } else {
        serde_json::to_vec(&canonical)
    }
}

/// Compose the bytes appended at the log's committed length: each event line with
/// exactly one trailing newline.
fn compose_events(lines: &[Vec<u8>]) -> Vec<u8> {
    lines
        .iter()
        .flat_map(|line| [line.as_slice(), b"\n"].concat())
        .collect()
}

/// The captured event log's committed length, the end of its last
/// newline-terminated line, from a tail that holds that line.
///
/// # Errors
///
/// [`CaptureError::TailOnlyCapture`] for a tail that holds only part of the
/// final line, which cannot say where that line starts.
fn captured_log_end(image: &RepositoryImage) -> Result<u64, MutationError> {
    match image.entry(&VirtualPath::EVENTS)? {
        RepositoryEntry::Absent => Ok(0),
        RepositoryEntry::FileTail { tail, .. } => {
            let final_line = tail
                .final_line()
                .ok_or(CaptureError::TailOnlyCapture(VirtualPath::EVENTS))?;
            Ok(tail.length() - final_line.len() as u64)
        }
        _ => Err(CaptureError::UnsafeCapturedOccupant(VirtualPath::EVENTS).into()),
    }
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

/// The single typed-to-byte finalizer for repository-owned records.
///
/// Given the captured `image`, the reused `context`, and closed semantic
/// `intents`, it produces one exact [`MaterializationPlan`]. When no transition occurs the plan carries an empty delta having
/// sampled neither an identifier nor the mutation timestamp, so a no-op emits no
/// bytes. Identifiers follow the frozen order — the indices of new issue ids
/// first (creation order), gate-run ids next, both derived by the caller and
/// reserved here, event ids last after canonical event ordering — and one mutation
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

/// The issue as persisted for `draft` created at `now`.
pub(crate) fn created_issue(draft: &Issue, now: DateTime<Utc>) -> Issue {
    let mut issue = draft.clone();
    issue.created_at = now;
    issue.updated_at = now;
    issue.first_ready_at = (issue.state == State::Ready).then_some(now);
    for gate in issue.gates_status.values_mut() {
        gate.updated_at = now;
    }
    issue
}

/// The issue a mutation persists, or its captured record for a no-op.
pub(crate) fn mutated_issue(
    image: &RepositoryImage,
    id: &IssueId,
    intents: &[MutationIntent],
    now: DateTime<Utc>,
) -> Result<Issue, MutationError> {
    let preimage =
        captured_issue(image, id)?.ok_or_else(|| MutationError::MissingIssue(id.clone()))?;
    let Some(issue) = intents.iter().find_map(|intent| match intent {
        MutationIntent::UpdateIssue { issue } if &issue.id == id => Some(issue.as_ref()),
        _ => None,
    }) else {
        return Ok(preimage);
    };
    Ok(updated_issue(&preimage, issue, intents, now))
}

fn updated_issue(
    preimage: &Issue,
    issue: &Issue,
    intents: &[MutationIntent],
    now: DateTime<Utc>,
) -> Issue {
    let mut updated = issue.clone();
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
                ) => issue_id == &updated.id && gate_key == key,
                _ => false,
            },
            _ => false,
        });
        if records_evidence || preimage.gates_status.get(key) != Some(state) {
            state.updated_at = now;
        }
    }
    updated.updated_at = now;
    updated
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
            let issue = created_issue(draft, context.timestamp());
            let id = issue.id.clone();
            actions.push(write_issue_action(image, &issue)?);
            pending_events.push(PendingEvent {
                phase: 0,
                tag: EventTag::IssueCreated,
                primary: Some(id.clone()),
                secondary: EventSecondaryIdentity::None,
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
                pending_events.push(PendingEvent {
                    phase: 0,
                    tag: EventTag::IssueCreated,
                    primary: Some(issue.id.clone()),
                    secondary: EventSecondaryIdentity::None,
                    ordinal: pending_events.len(),
                    body: EventBody::draft_issue_created(&issue),
                });
                if !issue.dependencies.is_empty() {
                    pending_events.push(PendingEvent {
                        phase: 1,
                        tag: EventTag::IssueUpdated,
                        primary: Some(issue.id.clone()),
                        secondary: EventSecondaryIdentity::None,
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

    // Gate-run drafts hold the next indices, having derived their ids at them.
    // They publish in (issue id, gate key) order.
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
    context.reserve(gate_runs.len());

    // Pass 2: claims of existing issues; no new identifiers.
    for intent in intents {
        if let MutationIntent::ClaimIssue { issue_id, agent } = intent {
            let mut issue = captured_issue(image, issue_id)?
                .ok_or_else(|| MutationError::MissingIssue(issue_id.clone()))?;
            if issue.assignee.as_ref() != Some(agent) {
                let now = context.timestamp();
                issue.assignee = Some(agent.clone());
                if issue.claimed_at.is_none() {
                    issue.claimed_at = Some(now);
                }
                issue.updated_at = now;
                actions.push(write_issue_action(image, &issue)?);
                pending_events.push(PendingEvent {
                    phase: 1,
                    tag: EventTag::IssueClaimed,
                    primary: Some(issue_id.clone()),
                    secondary: EventSecondaryIdentity::Assignee(agent.clone()),
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
            let updated = updated_issue(&preimage, issue, intents, context.timestamp());
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

    // Pass 3: gate-run records are published.
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
    for draft in gate_runs {
        let mut result = draft.clone();
        if result.origin == crate::domain::GateVerdictOrigin::Attested {
            let now = context.timestamp();
            result.started_at = now;
            result.completed_at = Some(now);
        }
        let directory = VirtualPath::gate_run_issue_directory(&result.issue_id);
        if matches!(image.entry(&directory)?, RepositoryEntry::Absent)
            && !actions.iter().any(|action| action.path() == &directory)
        {
            actions.push(RepositoryAction::CreateDirectory {
                path: directory,
                owner: RepositoryActionOwner::RepositoryStateMutation,
                expected: ExpectedPreimage::Absent,
            });
        }
        // The record is freshly named by a derived id, so it is absent.
        actions.push(RepositoryAction::WriteFile {
            path: VirtualPath::gate_run_record(&result.issue_id, &result.run_id),
            owner: RepositoryActionOwner::RepositoryStateMutation,
            expected: ExpectedPreimage::Absent,
            bytes: serialize_gate_run(&result)?,
            mode: FileMode::Regular,
        });
        // A run holding a reusable verdict becomes its key's reuse source. The
        // verdict record sorts after the run record it names, so it never names
        // an unpublished run.
        if let Some(digest) = result.reusable_digest() {
            let path = VirtualPath::gate_verdict_record(&result.gate_key, digest);
            for parent in path.ancestor_directories()? {
                if matches!(image.entry(&parent)?, RepositoryEntry::Absent)
                    && !actions.iter().any(|action| action.path() == &parent)
                {
                    actions.push(RepositoryAction::CreateDirectory {
                        path: parent,
                        owner: RepositoryActionOwner::RepositoryStateMutation,
                        expected: ExpectedPreimage::Absent,
                    });
                }
            }
            let record = GateVerdictRecord {
                issue_id: result.issue_id.clone(),
                run_id: result.run_id.clone(),
            };
            actions.push(RepositoryAction::WriteFile {
                expected: expected_of(image, &path)?,
                path,
                owner: RepositoryActionOwner::RepositoryStateMutation,
                bytes: serialize_gate_verdict(&record)?,
                mode: FileMode::Regular,
            });
        }
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

    // Deletions record their tombstones over the captured index preimage.
    if !index_deletions.is_empty() {
        actions.push(index_tombstone_action(image, &index_deletions)?);
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

/// Compose the audit-log append action for a set of pending events at the captured
/// `events.jsonl` committed length: admit the record the context's dispatch annotation
/// contributes, order them canonically, assign identifiers (via the context, in the
/// caller's frozen allocation order), stamp the single mutation timestamp, and produce
/// the appended bytes. Returns `None` when there are no events. This is the event pass
/// of [`finalize`], factored out so [`finalize_audit_append`] positions its append the
/// same way, and so the annotation record reaches both paths from one place.
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
    let committed_length = captured_log_end(image)?;

    let now = context.timestamp();
    // Assign identifiers in canonical event order and stamp the single mutation
    // timestamp.
    pending_events.sort_by(PendingEvent::compare);
    let events = pending_events
        .into_iter()
        .map(|pending| Event::new(context.allocate(), now, pending.body))
        .collect::<Vec<_>>();
    let lines = events
        .iter()
        .map(serialize_event)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(RepositoryAction::AppendEvents {
        owner: RepositoryActionOwner::RepositoryStateMutation,
        expected: expected_of(image, &VirtualPath::EVENTS)?,
        offset: committed_length,
        bytes: compose_events(&lines),
    }))
}

/// Compose the exact `events.jsonl` append action for repository-owned provenance
/// events emitted by a delta the record finalizer does not otherwise build — the
/// init/profile finalizers, whose deltas also carry asset/registry/record file
/// actions. Each `(phase, body)` pair is a drafted event body; this resets the context's frozen allocation order, then
/// assigns identifiers and the single mutation timestamp and positions the append
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

/// Derive the deterministic tombstone write from the captured preimage.
fn index_tombstone_action(
    image: &RepositoryImage,
    deleted_ids: &[IssueId],
) -> Result<RepositoryAction, MutationError> {
    let path = VirtualPath::INDEX;
    let expected = expected_of(image, &path)?;
    let bytes =
        captured_file_bytes(image, &path)?.ok_or_else(|| MutationError::MalformedRecord {
            path: path.clone(),
            reason: "the repository has no index".to_string(),
        })?;
    let mut index =
        super::RepositoryIndex::parse(bytes).map_err(|error| MutationError::MalformedRecord {
            path: path.clone(),
            reason: error.to_string(),
        })?;
    for id in deleted_ids {
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
fn event_identities(body: &EventBody) -> (Option<IssueId>, EventSecondaryIdentity) {
    let issue_id = body.issue_id().cloned();
    match body {
        EventBody::GatePassed { gate_key, .. }
        | EventBody::GateFailed { gate_key, .. }
        | EventBody::GateAdded { gate_key, .. }
        | EventBody::GateRemoved { gate_key, .. }
        | EventBody::GateDefinitionCreated { gate_key, .. }
        | EventBody::GateDefinitionUpdated { gate_key, .. }
        | EventBody::GateDefinitionRemoved { gate_key, .. } => {
            (issue_id, EventSecondaryIdentity::Gate(gate_key.clone()))
        }
        EventBody::IssueClaimed { assignee, .. } => {
            (issue_id, EventSecondaryIdentity::Assignee(assignee.clone()))
        }
        _ => (issue_id, EventSecondaryIdentity::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{GateRunStatus, GateState, GateStatus};
    use crate::repository_state::{
        CapturedBytes, RepositoryEntry, RepositoryImage, RepositoryIndex,
        SUPPORTED_INDEX_SCHEMA_VERSION,
    };
    use crate::test_harness::image::{synthetic_layout, ImageBuilder};
    use std::collections::HashMap;

    /// Fails when `serde_json`'s `preserve_order` feature is enabled.
    #[test]
    fn test_canonical_json_sorts_keys_a_struct_declares_out_of_order() {
        #[derive(Serialize)]
        struct Inner {
            second: u8,
            first: u8,
        }
        #[derive(Serialize)]
        struct Outer {
            zeta: u8,
            alpha: Inner,
        }

        let bytes = canonical_json(
            &Outer {
                zeta: 0,
                alpha: Inner {
                    second: 2,
                    first: 1,
                },
            },
            false,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            r#"{"alpha":{"first":1,"second":2},"zeta":0}"#
        );
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
            .map(|(key, value)| (key.parse().unwrap(), value))
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
            schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
            deleted_ids: Vec::new(),
        })
        .unwrap();
        let created_id = created_id();
        let image = ImageBuilder::new()
            .entry(VirtualPath::ISSUES, RepositoryEntry::Absent)
            .entry(VirtualPath::INDEX, file_entry(&index_bytes))
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::issue_record(&created_id),
                RepositoryEntry::Absent,
            )
            .build();
        let first = finalize(
            &synthetic_layout(),
            &image,
            &ctx(),
            &[MutationIntent::CreateIssue {
                draft: Box::new(first_draft),
            }],
        )
        .unwrap();
        let retry = finalize(
            &synthetic_layout(),
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
            deleted_ids: Vec::new(),
        })
        .unwrap();
        let created_id = created_id();
        ImageBuilder::new()
            .entry(VirtualPath::ISSUES, RepositoryEntry::Absent)
            .entry(VirtualPath::INDEX, file_entry(&index_bytes))
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::issue_record(&created_id),
                RepositoryEntry::Absent,
            )
            .build()
    }

    /// The exact bytes a delta appends to the event log.
    fn appended_event_bytes(delta: &RepositoryDelta) -> Option<Vec<u8>> {
        delta.appended_event_bytes().map(<[u8]>::to_vec)
    }

    /// The events a delta appends, as the event-log reader recovers them.
    fn appended_events(delta: &RepositoryDelta) -> Vec<Event> {
        appended_event_bytes(delta)
            .map(|bytes| crate::domain::event_log::parse_known_events(&bytes).unwrap())
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
        let plan = finalize(&synthetic_layout(), &image, &annotated_ctx(), &intents).unwrap();

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
            finalize(&synthetic_layout(), &image, &ctx(), &intents)
                .unwrap()
                .delta(),
        )
        .expect("an issue creation appends its own events");
        let annotated = appended_event_bytes(
            finalize(&synthetic_layout(), &image, &annotated_ctx(), &intents)
                .unwrap()
                .delta(),
        )
        .expect("an issue creation appends its own events");

        assert!(
            override_records(&crate::domain::event_log::parse_known_events(&unannotated).unwrap())
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
        let image = ImageBuilder::new()
            .entry(
                VirtualPath::issue_record(&issue.id),
                file_entry(&issue_bytes),
            )
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .build();
        let context = MutationContext::with_annotation(
            IdAuthority::random(),
            Box::new(PanicMutationClock),
            Some(override_annotation()),
        );
        let plan = finalize(
            &synthetic_layout(),
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
        let mode = FileMode::Regular;
        ImageBuilder::new()
            .entry(
                VirtualPath::EVENTS,
                RepositoryEntry::FileTail { tail, mode },
            )
            .build()
    }

    /// An audit append over a log read as its tail lands at the log's committed
    /// length and expects that tail, with no whole-log digest behind it.
    #[test]
    fn test_event_append_over_a_tail_read_offsets_at_its_committed_length_and_expects_the_tail() {
        let committed = b"{}\n".repeat(2 * crate::repository_state::EVENT_LOG_TAIL_WINDOW as usize);
        let log = [committed.as_slice(), b"{\"parti"].concat();
        let tail = crate::repository_state::EventLogTail::of_bytes(&log);
        let image = tail_image(tail.clone());

        let action = finalize_audit_append(&image, &ctx(), vec![(2, profile_lifecycle_event())])
            .unwrap()
            .expect("a non-empty audit append composes an action");

        let RepositoryAction::AppendEvents {
            offset, expected, ..
        } = action
        else {
            unreachable!("an audit append appends to the event log");
        };
        assert_eq!(offset, committed.len() as u64);
        assert_eq!(
            expected,
            ExpectedPreimage::FileTail {
                tail,
                mode: FileMode::Regular
            }
        );
    }

    /// A tail whose window starts inside the final line cannot determine the
    /// committed append offset.
    #[test]
    fn test_event_append_requires_a_tail_that_holds_the_final_line() {
        let window = crate::repository_state::EVENT_LOG_TAIL_WINDOW;
        let partial =
            crate::repository_state::EventLogTail::new(2 * window, vec![b'x'; window as usize]);
        let image = tail_image(partial.unwrap());

        assert!(matches!(
            finalize_audit_append(&image, &ctx(), vec![(2, profile_lifecycle_event())]),
            Err(MutationError::Capture(CaptureError::TailOnlyCapture(_)))
        ));
    }

    #[test]
    fn test_finalize_audit_append_publishes_the_dispatch_override_record_with_its_events() {
        // The second finalizer path composes its events through the same event pass,
        // so the dispatch fact reaches it without a second edit.
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .build();
        let action = finalize_audit_append(
            &image,
            &annotated_ctx(),
            vec![(2, profile_lifecycle_event())],
        )
        .unwrap()
        .expect("a non-empty audit append composes an action");
        let RepositoryAction::AppendEvents { bytes, .. } = &action else {
            unreachable!("an audit append appends to the event log");
        };
        let events = crate::domain::event_log::parse_known_events(bytes).unwrap();
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
            deleted_ids: Vec::new(),
        })
        .unwrap();
        let created_id = created_id();
        let image = ImageBuilder::new()
            .entry(VirtualPath::ISSUES, RepositoryEntry::Absent)
            .entry(VirtualPath::INDEX, file_entry(&index_bytes))
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::issue_record(&created_id),
                RepositoryEntry::Absent,
            )
            .build();

        // Deliberately prove the finalizer owns these values rather than accepting
        // caller timestamps embedded in the draft.
        for gate in draft.gates_status.values_mut() {
            gate.updated_at = DateTime::parse_from_rfc3339("2001-02-03T04:05:06Z")
                .unwrap()
                .with_timezone(&Utc);
        }
        let plan = finalize(
            &synthetic_layout(),
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
    fn test_serialize_issue_is_byte_identical_over_its_own_output() {
        let bytes = serialize_issue(&map_order_draft(false)).unwrap();
        let decoded: Issue = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(serialize_issue(&decoded).unwrap(), bytes);
    }

    #[test]
    fn test_canonical_serialization_preserves_repository_record_shapes() {
        let issue_bytes = serialize_issue(&map_order_draft(false)).unwrap();
        assert!(issue_bytes.starts_with(b"{\n"));
        assert!(!issue_bytes.ends_with(b"\n"));

        let gate_run = GateRunResult::attestation(
            &crate::test_utils::gate_run_id(1),
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

    #[test]
    fn test_update_issue_stamps_changed_gate_state_from_mutation_context() {
        let id = &crate::test_utils::issue_id(0x1111_4111_8111_1111_1111_1111_1111_1111);
        let old_time = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut preimage = seeded_issue(id, None);
        preimage.gates_status.insert(
            "tests".parse().unwrap(),
            GateState {
                status: GateStatus::Pending,
                updated_by: None,
                updated_at: old_time,
                run_id: None,
            },
        );
        let mut updated = preimage.clone();
        updated.gates_status.insert(
            "tests".parse().unwrap(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("agent:reviewer".parse().unwrap()),
                // Checker/manual callers submit no lifecycle timestamp.
                updated_at: DateTime::default(),
                run_id: None,
            },
        );
        let bytes = serialize_issue(&preimage).unwrap();
        let image = ImageBuilder::new()
            .entry(VirtualPath::issue_record(id), file_entry(&bytes))
            .build();
        let intent = MutationIntent::UpdateIssue {
            issue: Box::new(updated),
        };
        let first = finalize(
            &synthetic_layout(),
            &image,
            &ctx(),
            std::slice::from_ref(&intent),
        )
        .unwrap();
        let retry = finalize(&synthetic_layout(), &image, &ctx(), &[intent]).unwrap();
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
        assert_eq!(
            persisted.gates_status[&"tests".parse().unwrap()].updated_at,
            fixed_instant()
        );
        assert_eq!(persisted.updated_at, fixed_instant());
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

    /// An assignment made outside jit leaves no claim event behind, and a claim
    /// does not repair one.
    #[test]
    fn test_finalize_claim_of_issue_already_assigned_outside_jit_appends_nothing() {
        let agent: Assignee = "agent:worker-1".parse().unwrap();
        let issue = seeded_issue(
            &crate::test_utils::issue_id(0x4444_4444_8444_4444_4444_4444_4444_4444),
            Some(agent.clone()),
        );
        let issue_bytes = serialize_issue(&issue).unwrap();
        let image = ImageBuilder::new()
            .entry(
                VirtualPath::issue_record(&issue.id),
                file_entry(&issue_bytes),
            )
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .build();
        let context = MutationContext::new(IdAuthority::random(), Box::new(PanicMutationClock));
        let plan = finalize(
            &synthetic_layout(),
            &image,
            &context,
            &[MutationIntent::ClaimIssue {
                issue_id: issue.id.clone(),
                agent,
            }],
        )
        .unwrap();
        assert!(plan.delta().actions().is_empty());
        assert!(
            context.ids.sampled_seed().is_none(),
            "a no-op claim draws no identifier"
        );
    }

    /// The decision follows the assignee alone: a tail no parser accepts changes
    /// neither outcome.
    #[test]
    fn test_finalize_claim_decides_from_the_assignee_over_an_unparseable_log_tail() {
        let agent: Assignee = "agent:worker-1".parse().unwrap();
        let tail = crate::repository_state::EventLogTail::of_bytes(b"not an event\n");
        let claim = |assignee: Option<Assignee>| {
            let issue = seeded_issue(
                &crate::test_utils::issue_id(0x2222_4222_8222_2222_2222_2222_2222_2222),
                assignee,
            );
            let image = ImageBuilder::new()
                .entry(
                    VirtualPath::issue_record(&issue.id),
                    file_entry(&serialize_issue(&issue).unwrap()),
                )
                .entry(
                    VirtualPath::EVENTS,
                    RepositoryEntry::FileTail {
                        tail: tail.clone(),
                        mode: FileMode::Regular,
                    },
                )
                .build();
            finalize(
                &synthetic_layout(),
                &image,
                &ctx(),
                &[MutationIntent::ClaimIssue {
                    issue_id: issue.id,
                    agent: agent.clone(),
                }],
            )
            .unwrap()
        };

        assert!(claim(Some(agent.clone())).delta().actions().is_empty());
        assert_eq!(claim(None).delta().actions().len(), 2);
    }

    #[test]
    fn test_claim_fresh_emits_issue_and_event() {
        let agent: Assignee = "agent:worker-1".parse().unwrap();
        let issue = seeded_issue(
            &crate::test_utils::issue_id(0x3333_4333_8333_3333_3333_3333_3333_3333),
            None,
        );
        let issue_bytes = serialize_issue(&issue).unwrap();
        let image = ImageBuilder::new()
            .entry(
                VirtualPath::issue_record(&issue.id),
                file_entry(&issue_bytes),
            )
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .build();
        let intent = MutationIntent::ClaimIssue {
            issue_id: issue.id.clone(),
            agent: agent.clone(),
        };
        let context = ctx();
        let first = finalize(
            &synthetic_layout(),
            &image,
            &context,
            std::slice::from_ref(&intent),
        )
        .unwrap();
        let retry = finalize(&synthetic_layout(), &image, &context, &[intent]).unwrap();
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
        let plan = finalize(
            &synthetic_layout(),
            &ImageBuilder::new().build(),
            &context,
            &[],
        )
        .unwrap();

        assert!(plan.delta().actions().is_empty());
        assert!(
            context.ids.sampled_seed().is_none(),
            "empty intents must not draw the production identifier seed"
        );
    }

    #[test]
    fn test_finalize_record_event_reuses_identity_time_and_bytes_on_retry() {
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .build();
        let intent = MutationIntent::RecordEvent {
            phase: 0,
            event: Box::new(EventBody::draft_gate_definition_created(
                "cargo-ci".parse().unwrap(),
            )),
        };
        let context = ctx();
        let first = finalize(
            &synthetic_layout(),
            &image,
            &context,
            std::slice::from_ref(&intent),
        )
        .unwrap();
        let retry = finalize(&synthetic_layout(), &image, &context, &[intent]).unwrap();

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
    fn test_finalize_gate_run_reserves_its_derived_id_and_serializes_attestation() {
        let context = ctx();
        let run_id = context.gate_run_id_at(0);
        let issue = crate::test_utils::issue_id(7);
        let image = ImageBuilder::new()
            .entry(VirtualPath::GATE_RUNS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::gate_run_issue_directory(&issue),
                RepositoryEntry::Absent,
            )
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .build();
        let draft = GateRunResult::attestation(
            &run_id,
            &issue,
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
        let first = finalize(&synthetic_layout(), &image, &context, &intents).unwrap();
        let retry = finalize(&synthetic_layout(), &image, &context, &intents).unwrap();

        assert_eq!(
            serde_json::to_vec(first.delta()).unwrap(),
            serde_json::to_vec(retry.delta()).unwrap(),
            "a retry keeps the run record bytes"
        );
        let result_path = VirtualPath::gate_run_record(&issue, &run_id);
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

    fn verdict_digest() -> crate::domain::repository_inputs::InputsDigest {
        crate::domain::repository_inputs::InputsDigest::try_from("c".repeat(64)).unwrap()
    }

    /// A run of `cargo-ci` its derived id names, executed over [`verdict_digest`].
    fn executed_run(context: &MutationContext, status: GateRunStatus) -> GateRunResult {
        let mut run = GateRunResult::attestation(
            &context.gate_run_id_at(0),
            &crate::test_utils::issue_id(7),
            &"cargo-ci".parse().unwrap(),
            crate::declarations::GateStage::Postcheck,
            GateStatus::Passed,
            None,
        );
        run.origin = crate::domain::GateVerdictOrigin::Executed;
        run.status = status;
        run.inputs_digest = Some(verdict_digest());
        run
    }

    /// An image holding `gate-runs` and, when `prior` is set, a verdict record
    /// for `cargo-ci` over [`verdict_digest`] naming `prior`.
    fn verdict_image(
        context: &MutationContext,
        prior: Option<&GateVerdictRecord>,
    ) -> RepositoryImage {
        let gate_key = "cargo-ci".parse().unwrap();
        let run_id = context.gate_run_id_at(0);
        let directory = || RepositoryEntry::Directory {
            identity: crate::repository_state::EntryIdentity::for_bytes("dir", b"dir").unwrap(),
            mode: FileMode::Regular,
        };
        let issue = crate::test_utils::issue_id(7);
        let builder = ImageBuilder::new()
            .entry(VirtualPath::GATE_RUNS, directory())
            .entry(
                VirtualPath::gate_run_issue_directory(&issue),
                RepositoryEntry::Absent,
            )
            .entry(
                VirtualPath::gate_run_record(&issue, &run_id),
                RepositoryEntry::Absent,
            );
        let record = VirtualPath::gate_verdict_record(&gate_key, &verdict_digest());
        match prior {
            None => builder
                .entry(VirtualPath::GATE_VERDICTS, RepositoryEntry::Absent)
                .entry(
                    VirtualPath::gate_verdict_directory(&gate_key),
                    RepositoryEntry::Absent,
                )
                .entry(record, RepositoryEntry::Absent),
            Some(prior) => builder
                .entry(VirtualPath::GATE_VERDICTS, directory())
                .entry(VirtualPath::gate_verdict_directory(&gate_key), directory())
                .entry(record, file_entry(&serialize_gate_verdict(prior).unwrap())),
        }
        .build()
    }

    /// The delta actions under `gate-runs/verdicts`.
    fn verdict_actions(delta: &RepositoryDelta) -> Vec<&RepositoryAction> {
        delta
            .actions()
            .iter()
            .filter(|action| {
                action
                    .path()
                    .relative()
                    .as_path()
                    .starts_with(VirtualPath::GATE_VERDICTS.relative().as_path())
            })
            .collect()
    }

    fn record_run(
        image: &RepositoryImage,
        context: &MutationContext,
        run: GateRunResult,
    ) -> RepositoryDelta {
        finalize(
            &synthetic_layout(),
            image,
            context,
            &[MutationIntent::RecordGateRun {
                draft: Box::new(run),
            }],
        )
        .unwrap()
        .delta()
        .clone()
    }

    #[test]
    fn test_finalize_gate_run_creates_an_issue_directory_only_when_captured_absent() {
        let context = ctx();
        let (recorded, fresh) = (
            crate::test_utils::issue_id(7),
            crate::test_utils::issue_id(8),
        );
        let directory = RepositoryEntry::Directory {
            identity: crate::repository_state::EntryIdentity::for_bytes("dir", b"dir").unwrap(),
            mode: FileMode::Regular,
        };
        let runs = [(&recorded, 0), (&fresh, 1), (&fresh, 2)].map(|(issue, index)| {
            GateRunResult::attestation(
                &context.gate_run_id_at(index),
                issue,
                &"cargo-ci".parse().unwrap(),
                crate::declarations::GateStage::Postcheck,
                GateStatus::Passed,
                None,
            )
        });
        let image = runs
            .iter()
            .fold(
                ImageBuilder::new()
                    .entry(VirtualPath::GATE_RUNS, directory.clone())
                    .entry(VirtualPath::gate_run_issue_directory(&recorded), directory)
                    .entry(
                        VirtualPath::gate_run_issue_directory(&fresh),
                        RepositoryEntry::Absent,
                    ),
                |builder, run| {
                    builder.entry(
                        VirtualPath::gate_run_record(&run.issue_id, &run.run_id),
                        RepositoryEntry::Absent,
                    )
                },
            )
            .build();
        let intents = runs
            .iter()
            .map(|run| MutationIntent::RecordGateRun {
                draft: Box::new(run.clone()),
            })
            .collect::<Vec<_>>();

        let plan = finalize(&synthetic_layout(), &image, &context, &intents).unwrap();

        let created = plan
            .delta()
            .actions()
            .iter()
            .filter_map(|action| match action {
                RepositoryAction::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(created, [VirtualPath::gate_run_issue_directory(&fresh)]);
        assert!(runs
            .iter()
            .all(|run| plan.delta().actions().iter().any(|action| matches!(
                action,
                RepositoryAction::WriteFile { path, expected: ExpectedPreimage::Absent, .. }
                    if *path == VirtualPath::gate_run_record(&run.issue_id, &run.run_id)
            ))));
    }

    #[test]
    fn test_finalize_gate_run_with_a_reusable_verdict_writes_its_verdict_record() {
        let context = ctx();
        let image = verdict_image(&context, None);
        for status in [GateRunStatus::Passed, GateRunStatus::Failed] {
            let run = executed_run(&context, status);
            let delta = record_run(&image, &context, run.clone());

            let record = VirtualPath::gate_verdict_record(&run.gate_key, &verdict_digest());
            let expected = GateVerdictRecord {
                issue_id: run.issue_id.clone(),
                run_id: run.run_id.clone(),
            };
            assert!(
                matches!(
                    verdict_actions(&delta).as_slice(),
                    [
                        RepositoryAction::CreateDirectory { path: verdicts, expected: ExpectedPreimage::Absent, .. },
                        RepositoryAction::CreateDirectory { path: gate, expected: ExpectedPreimage::Absent, .. },
                        RepositoryAction::WriteFile { path, bytes, expected: ExpectedPreimage::Absent, .. },
                    ] if *verdicts == VirtualPath::GATE_VERDICTS
                        && *gate == VirtualPath::gate_verdict_directory(&run.gate_key)
                        && *path == record
                        && serde_json::from_slice::<GateVerdictRecord>(bytes).unwrap() == expected
                ),
                "{status:?}: {:?}",
                delta.actions()
            );
        }
    }

    #[test]
    fn test_finalize_gate_run_with_a_reusable_verdict_replaces_the_keys_prior_record() {
        let context = ctx();
        let prior = GateVerdictRecord {
            issue_id: crate::test_utils::issue_id(3),
            run_id: crate::test_utils::gate_run_id(9),
        };
        let image = verdict_image(&context, Some(&prior));
        let run = executed_run(&context, GateRunStatus::Passed);
        let record = VirtualPath::gate_verdict_record(&run.gate_key, &verdict_digest());

        let delta = record_run(&image, &context, run.clone());

        assert!(
            matches!(
                verdict_actions(&delta).as_slice(),
                [RepositoryAction::WriteFile { path, bytes, expected, .. }]
                    if *path == record
                        && *expected == ExpectedPreimage::of(image.entry(&record).unwrap())
                        && serde_json::from_slice::<GateVerdictRecord>(bytes).unwrap().run_id == run.run_id
            ),
            "{:?}",
            delta.actions()
        );
    }

    #[test]
    fn test_finalize_gate_run_without_a_reusable_verdict_writes_no_verdict_record() {
        let context = ctx();
        let image = verdict_image(&context, None);
        let executed = executed_run(&context, GateRunStatus::Passed);
        let runs = [
            GateRunResult {
                inputs_digest: None,
                ..executed.clone()
            },
            GateRunResult {
                status: GateRunStatus::Error,
                ..executed.clone()
            },
            GateRunResult {
                origin: crate::domain::GateVerdictOrigin::Reused {
                    source_issue: crate::test_utils::issue_id(3),
                    source_run: crate::test_utils::gate_run_id(9),
                },
                ..executed.clone()
            },
            GateRunResult {
                origin: crate::domain::GateVerdictOrigin::Attested,
                ..executed
            },
        ];
        for run in runs {
            let delta = record_run(&image, &context, run.clone());
            assert!(
                verdict_actions(&delta).is_empty(),
                "{run:?}: {:?}",
                delta.actions()
            );
            assert!(delta
                .actions()
                .iter()
                .any(|action| action.path()
                    == &VirtualPath::gate_run_record(&run.issue_id, &run.run_id)));
        }
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
        let first = finalize(&synthetic_layout(), &image, &context, &intents).unwrap();
        let retry = finalize(&synthetic_layout(), &image, &context, &intents).unwrap();
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
            deleted_ids: Vec::new(),
        })
        .unwrap();
        let image = draft_ids
            .iter()
            .fold(
                ImageBuilder::new()
                    .entry(VirtualPath::ISSUES, RepositoryEntry::Absent)
                    .entry(VirtualPath::INDEX, file_entry(&index_bytes))
                    .entry(VirtualPath::EVENTS, RepositoryEntry::Absent),
                |builder, id| builder.entry(VirtualPath::issue_record(id), RepositoryEntry::Absent),
            )
            .build();
        let plan = finalize(
            &synthetic_layout(),
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
    fn test_create_issue_stamps_lifecycle_and_writes_no_index() {
        let index_bytes = serde_json::to_vec_pretty(&RepositoryIndex {
            schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
            deleted_ids: Vec::new(),
        })
        .unwrap();
        let draft = {
            let mut issue = Issue::draft(created_id(), "New".into(), "Body".into());
            issue.state = State::Ready;
            issue
        };
        let image = ImageBuilder::new()
            .entry(VirtualPath::ISSUES, RepositoryEntry::Absent)
            .entry(VirtualPath::INDEX, file_entry(&index_bytes))
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            // The draft carries its id; seed that id's absent slot.
            .entry(
                VirtualPath::issue_record(&created_id()),
                RepositoryEntry::Absent,
            )
            .build();
        let delta = finalize(
            &synthetic_layout(),
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
        assert!(!delta
            .delta()
            .actions()
            .iter()
            .any(|action| action.path() == &VirtualPath::INDEX));
    }

    #[test]
    fn test_delete_issue_removes_record_and_records_its_tombstone() {
        let issue = seeded_issue(
            &crate::test_utils::issue_id(0x1111_4111_8111_1111_1111_1111_1111_1111),
            None,
        );
        let issue_bytes = serialize_issue(&issue).unwrap();
        let index_bytes = canonical_json(
            &RepositoryIndex {
                schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
                deleted_ids: Vec::new(),
            },
            true,
        )
        .unwrap();
        let image = ImageBuilder::new()
            .entry(
                VirtualPath::issue_record(&issue.id),
                file_entry(&issue_bytes),
            )
            .entry(VirtualPath::INDEX, file_entry(&index_bytes))
            .build();

        let plan = finalize(
            &synthetic_layout(),
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
        assert_eq!(index.deleted_ids, [issue.id]);
    }

    #[test]
    fn test_delete_issue_rejects_duplicate_deleted_index_without_a_plan() {
        let issue = seeded_issue(
            &crate::test_utils::issue_id(0x1111_4111_8111_1111_1111_1111_1111_1111),
            None,
        );
        let issue_bytes = serialize_issue(&issue).unwrap();
        let index_bytes = serde_json::to_vec(&serde_json::json!({
            "schema_version": SUPPORTED_INDEX_SCHEMA_VERSION,
            "deleted_ids": [crate::test_utils::issue_id(1), crate::test_utils::issue_id(1)]
        }))
        .unwrap();
        let image = ImageBuilder::new()
            .entry(
                VirtualPath::issue_record(&issue.id),
                file_entry(&issue_bytes),
            )
            .entry(VirtualPath::INDEX, file_entry(&index_bytes))
            .build();

        let error = finalize(
            &synthetic_layout(),
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
        let image = ImageBuilder::new()
            .entry(VirtualPath::issue_record(&issue.id), file_entry(&bytes))
            .build();

        let plan = finalize(
            &synthetic_layout(),
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

    /// The finalizer's event-log action over `image`: its offset and bytes.
    fn event_append(image: &RepositoryImage, intents: &[MutationIntent]) -> (u64, Vec<u8>) {
        let plan = finalize(&synthetic_layout(), image, &ctx(), intents).unwrap();
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
    fn test_finalize_appends_at_the_committed_length_leaving_every_line_terminated() {
        let prior = serialize_event(&crate::test_utils::stamped_event(
            EventBody::draft_gate_definition_created("prior".parse().unwrap()),
            "e0",
            fixed_instant(),
        ))
        .unwrap();
        let terminated = [prior.as_slice(), b"\n"].concat();
        let unterminated = [terminated.as_slice(), b"{\"parti"].concat();
        // Final lines longer than the tail window, read back to their start.
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
        let long_valid = [terminated.as_slice(), &long].concat();
        let long_unterminated = [
            terminated.as_slice(),
            &vec![b'x'; 2 * crate::repository_state::EVENT_LOG_TAIL_WINDOW as usize],
        ]
        .concat();
        let tail = |log: Vec<u8>| {
            let image = tail_image(crate::repository_state::EventLogTail::of_bytes(&log));
            (log, image)
        };
        let captured = |log: Vec<u8>| {
            let image = ImageBuilder::new()
                .entry(VirtualPath::EVENTS, file_entry(&log))
                .build();
            (log, image)
        };
        let record = MutationIntent::RecordEvent {
            phase: 0,
            event: Box::new(EventBody::draft_gate_definition_created(
                "cargo-ci".parse().unwrap(),
            )),
        };
        let cases = [Vec::new(), terminated.clone(), prior.clone(), unterminated]
            .map(captured)
            .into_iter()
            .chain([long_valid, long_unterminated].map(tail));
        for (log, image) in cases {
            let (offset, appended) = event_append(&image, std::slice::from_ref(&record));

            let committed = log
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |newline| newline + 1);
            assert_eq!(offset, committed as u64, "log {log:?}");
            let result = [&log[..committed], &appended].concat();
            assert!(
                result.ends_with(b"\n") && !appended.starts_with(b"\n"),
                "no separator, and every line is terminated: {result:?}"
            );
            assert_eq!(
                crate::domain::event_log::parse_known_events(&result).unwrap(),
                [
                    crate::domain::event_log::parse_known_events(&log).unwrap(),
                    crate::domain::event_log::parse_known_events(&appended).unwrap(),
                ]
                .concat(),
                "the log read after the append is the committed events then the appended ones"
            );
        }
    }

    fn profile_lifecycle_event() -> EventBody {
        EventBody::draft_profile_lifecycle(
            crate::domain::ProfileLifecycleOperation::Apply,
            vec![crate::domain::ProfileLifecycleProfile {
                id: "example"
                    .try_into()
                    .expect("the profile event has a canonical identifier"),
                version: semver::Version::new(1, 0, 0),
                status: crate::domain::ProfileLifecycleStatus::Installed,
                variables: Default::default(),
            }],
        )
    }

    fn log_with_unterminated_fragment() -> Vec<u8> {
        let event = crate::test_utils::stamped_event(
            EventBody::draft_issue_completed(crate::test_utils::issue_id(1)),
            "e0",
            fixed_instant(),
        );
        [serialize_event(&event).unwrap().as_slice(), b"\n{\"parti"].concat()
    }

    #[test]
    fn test_finalize_keeps_canonical_event_order_when_tail_is_unterminated() {
        let image = ImageBuilder::new()
            .entry(
                VirtualPath::EVENTS,
                file_entry(&log_with_unterminated_fragment()),
            )
            .build();
        let record = |event| MutationIntent::RecordEvent {
            phase: 0,
            event: Box::new(event),
        };
        let (_, appended) = event_append(
            &image,
            &[
                record(EventBody::draft_gate_definition_created(
                    "cargo-ci".parse().unwrap(),
                )),
                record(profile_lifecycle_event()),
            ],
        );

        let events = crate::domain::event_log::parse_known_events(&appended).unwrap();
        assert!(matches!(
            events[0].body(),
            EventBody::GateDefinitionCreated { .. }
        ));
        assert!(matches!(
            events[1].body(),
            EventBody::ProfileLifecycle { .. }
        ));
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
