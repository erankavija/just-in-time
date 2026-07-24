//! Typed mutation identity, time authority, and the sole typed-to-byte finalizer.
//!
//! Commands submit timestamp-free, identifier-free semantic [`MutationIntent`]s.
//! A [`MutationContext`] — created exactly once per command operation, before
//! its conflict-retry loop — supplies deterministic identifiers (from a single seed drawn
//! once on the first allocation) and one mutation timestamp (sampled once, on the
//! first transition), so retries reuse identical record identities and times while
//! a no-op mutation samples neither identity nor time. The
//! finalizer serializes every repository-owned record (issue upserts/deletes,
//! `index.json` membership, gate-run/audit artifacts, and events) into one exact
//! [`RepositoryDelta`] published through the recovered store, preserving
//! `@/inv/event-log` and `@/inv/atomic-writes`.
//!
//! This module performs no filesystem I/O and imports only `domain` value types
//! and the pure `repository_state` vocabulary.

use super::{
    CaptureError, DeltaError, ExpectedPreimage, FileMode, MaterializationIntent,
    MaterializationPlan, PlanHashError, RepositoryAction, RepositoryDelta, RepositoryEntry,
    RepositoryImage, RepositoryLayout, RepositoryLayoutError, RepositorySeed, RepositorySeedKind,
    RootRelativePath, SeedError, VirtualPath,
};
use crate::declarations::GateRegistry;
use crate::domain::{Assignee, Event, GateRunResult, GateStatus, Issue, State};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::collections::BTreeMap;

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

    /// The seed if already determined without drawing the random source: a fixed
    /// seed, or a lazy seed already sampled by an allocation. `None` means no
    /// identifier has been allocated, so no-op finalization can hash its plan
    /// without forcing a sample.
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
        uuid::Uuid::from_bytes(bytes).to_string()
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
    sampled: Cell<Option<DateTime<Utc>>>,
    next_index: Cell<u64>,
}

impl MutationContext {
    /// Construct a context from an injected authority and clock.
    pub fn new(ids: IdAuthority, clock: Box<dyn MutationClock>) -> Self {
        Self {
            ids,
            clock,
            sampled: Cell::new(None),
            next_index: Cell::new(0),
        }
    }

    /// The production context: a lazily-drawn random seed and the system clock.
    /// A no-op mutation allocates no identifier and stamps no transition, so it
    /// draws neither the random source nor the clock.
    pub fn production() -> Self {
        Self::new(IdAuthority::random(), Box::new(SystemMutationClock))
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

    /// Derive an identifier at the finalizer's frozen allocation index without
    /// advancing that order. Command orchestration uses this only to capture the
    /// expected-absent paths of records whose finalizer-assigned ids are part of
    /// their path, and to return those ids after publication.
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

    /// Allocate the next identifier in the frozen order.
    fn allocate(&self) -> String {
        let index = self.next_index.get();
        self.next_index.set(index + 1);
        self.ids.uuid_at(index)
    }

    /// Close the identity seed and exact semantic intents into the typed seed
    /// consumed by the repository plan-hash API. The seed enters the hash only
    /// once it has been drawn — a fixed seed, or a production seed sampled by an
    /// allocation in this finalize. A no-op allocates nothing, so its production
    /// seed stays undrawn and is omitted, keeping no-op finalization free of any
    /// identity or time sampling. Repeated finalization with this context reuses
    /// its sampled seed, so equivalent plans hash identically on both backends.
    pub(crate) fn repository_seed(
        &self,
        intents: &[MutationIntent],
    ) -> Result<RepositorySeed, MutationError> {
        let context_seed = self
            .ids
            .sampled_seed()
            .map(|seed| seed.to_vec())
            .unwrap_or_default();
        RepositorySeed::new(
            RepositorySeedKind::Command {
                name: "repository-state-mutation-v1".to_string(),
            },
            BTreeMap::new(),
            BTreeMap::from([
                ("context_seed".to_string(), context_seed),
                ("intents".to_string(), canonical_json(intents, false)?),
            ]),
        )
        .map_err(Into::into)
    }
}

/// One repository-owned semantic mutation, free of identifiers and timestamps.
///
/// The finalizer assigns identity and applies the mutation timestamp; callers
/// never stamp either. The set is closed: a new record class extends this enum.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MutationIntent {
    /// Create a new issue. The finalizer assigns the id and stamps `created_at`,
    /// `updated_at`, and `first_ready_at` (when initially `Ready`); every other
    /// field is taken verbatim from `draft`.
    CreateIssue {
        /// Semantic issue whose id/lifecycle timestamps are assigned by the
        /// finalizer; all other fields are authoritative.
        draft: Box<Issue>,
    },
    /// Create a prevalidated issue batch and wire all intra-batch dependency
    /// relationships in the same delta. Dependency pairs are
    /// `(dependent_index, dependency_index)` into `drafts`; the finalizer
    /// resolves them after allocating every issue id.
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
        issue_id: String,
        /// Claiming agent identity.
        agent: Assignee,
    },
    /// Semantic full-issue update of an issue that ALREADY EXISTS in the captured
    /// image. The finalizer stamps `updated_at` from the mutation clock and derives
    /// `created_at`/`first_ready_at` from the captured preimage (they are not the
    /// caller's to set); every other field is taken verbatim from `issue`. Index
    /// membership is unchanged (no id is allocated), and a target absent from the
    /// captured image is a typed [`MutationError::MissingIssue`] — never an implicit
    /// create.
    UpdateIssue {
        /// Semantic issue whose id must match a captured issue; its lifecycle
        /// timestamps are reconciled against the preimage by the finalizer.
        issue: Box<Issue>,
    },
    /// Repair reconstructed lifecycle history on an existing issue. Unlike an
    /// ordinary semantic update, the caller-provided lifecycle timestamps are
    /// authoritative evidence derived from the captured audit log. Creation
    /// identity and `updated_at` remain byte-for-byte values from the captured
    /// issue: historical reconstruction changes only the three lifecycle fields.
    RepairIssueLifecycle {
        /// Existing issue carrying reconstructed historical lifecycle fields.
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
    /// must be absent in the captured image; custom presets never overwrite one
    /// another or shadow a builtin preset.
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
        issue_id: String,
    },
    /// Record one gate-run artifact. The finalizer assigns `run_id` and preserves
    /// external checker `started_at`/`completed_at` evidence verbatim.
    RecordGateRun {
        /// Semantic gate-run result whose `run_id` is assigned by the finalizer.
        draft: Box<GateRunResult>,
    },
    /// Append one repository-owned provenance/audit event. The finalizer assigns
    /// the event id and stamps its timestamp.
    RecordEvent {
        /// Deterministic ordering phase within the mutation.
        phase: u8,
        /// Event whose id/timestamp are assigned by the finalizer.
        event: Box<Event>,
    },
}

/// One pending event awaiting deterministic ordering and identity assignment.
struct PendingEvent {
    phase: u8,
    tag: String,
    primary: String,
    secondary: String,
    ordinal: usize,
    event: Event,
}

impl PendingEvent {
    fn sort_key(&self) -> (u8, &str, &str, &str, usize) {
        (
            self.phase,
            self.tag.as_str(),
            self.primary.as_str(),
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
    /// The closed semantic seed was invalid.
    #[error(transparent)]
    Seed(#[from] SeedError),
    /// The complete semantic plan could not be hashed.
    #[error(transparent)]
    PlanHash(#[from] PlanHashError),
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
    MissingIssue(String),
    /// The captured `events.jsonl` ends in an uncertified torn tail and this
    /// mutation carries no `ProfileApplied` marker to certify it, so appending
    /// would produce a log the reader rejects.
    #[error(
        "captured events.jsonl has an uncertified torn tail and the mutation has no \
         ProfileApplied marker to certify it; refusing to append (@/inv/event-log)"
    )]
    UncertifiedTornTail,
    /// A custom preset failed structural validation, or the builtin preset set
    /// could not be loaded; carries the underlying validator's forwarded message.
    #[error("invalid custom gate preset: {0}")]
    InvalidGatePreset(String),
    /// A custom preset name collides with a builtin preset.
    #[error("invalid custom gate preset: '{0}' collides with a builtin preset")]
    GatePresetCollidesWithBuiltin(String),
    /// A custom-preset ancestor path is occupied by a non-directory.
    #[error("invalid custom gate preset: custom preset parent '{0}' is not a directory")]
    GatePresetParentNotDirectory(String),
    /// The canonical custom-preset target was already occupied.
    #[error("custom gate preset '{0}' already exists")]
    GatePresetAlreadyExists(String),
}

/// Owner identity stamped on finalizer-produced actions.
const OWNER: &str = "repository-state-mutation";

/// Placeholder timestamp for a finalizer-built event before identity assignment.
///
/// Semantic event drafts carry this same empty identity/epoch convention; the
/// finalizer overwrites it via [`Event::assign_identity`] with the single mutation
/// timestamp. The sentinel therefore never reaches serialized bytes.
fn sentinel_time() -> DateTime<Utc> {
    DateTime::from_timestamp(0, 0).expect("epoch is representable")
}

/// The canonical path for one issue record.
fn issue_path(id: &str) -> Result<VirtualPath, RepositoryLayoutError> {
    VirtualPath::data(format!("issues/{id}.json"))
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
    let Some(listing) = image.listing_fingerprints().get(&root) else {
        return Ok(None);
    };
    listing
        .children()
        .keys()
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

/// Compose the exact `events.jsonl` bytes: preserve every prefix byte, add one
/// separator newline only when a non-empty prefix lacks one, then append each
/// event line with exactly one trailing newline.
fn compose_events(prefix: &[u8], lines: &[Vec<u8>]) -> Vec<u8> {
    let mut out = prefix.to_vec();
    if !out.is_empty() && !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    for line in lines {
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    out
}

/// Read the captured bytes of a `Data(...)` file entry, or `None` when absent.
fn captured_file_bytes<'a>(
    image: &'a RepositoryImage,
    path: &VirtualPath,
) -> Result<Option<&'a [u8]>, MutationError> {
    Ok(image.file_bytes(path)?)
}

/// Parse the captured issue at `id`, or `None` when the issue file is absent.
fn captured_issue(image: &RepositoryImage, id: &str) -> Result<Option<Issue>, MutationError> {
    let path = issue_path(id)?;
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
    crate::domain::parse_known_events(text).map_err(|error| MutationError::MalformedRecord {
        path: path.clone(),
        reason: error.to_string(),
    })
}

/// Whether the captured event tail already reflects `agent`'s claim of `issue_id`
/// as the current assignment: the most recent assignment-affecting event for the
/// issue is an `IssueClaimed` by `agent` with no later release.
fn event_tail_reflects_claim(events: &[Event], issue_id: &str, agent: &Assignee) -> bool {
    events
        .iter()
        .rev()
        .find_map(|event| match event {
            Event::IssueClaimed {
                issue_id: id,
                assignee,
                ..
            } if id == issue_id => Some(assignee == agent),
            Event::IssueReleased { issue_id: id, .. } if id == issue_id => Some(false),
            _ => None,
        })
        .unwrap_or(false)
}

/// The single typed-to-byte finalizer for repository-owned records.
///
/// Given the captured `image`, the reused `context`, and closed semantic
/// `intents`, it produces one exact [`MaterializationPlan`]. Its semantic hash
/// covers the complete captured image, the unchanged context seed, the closed
/// intent representation, [`MaterializationIntent::SemanticMutation`], and the
/// final delta. When no transition occurs the plan carries an empty delta having
/// sampled neither an identifier nor the mutation timestamp, so a no-op emits no
/// bytes. Identifiers are allocated in the frozen order — new issue ids first
/// (creation order), gate-run/record ids next, event ids last after canonical
/// event ordering — and one mutation timestamp is applied to every transition
/// that occurs.
pub fn finalize(
    layout: &RepositoryLayout,
    image: &RepositoryImage,
    context: &MutationContext,
    intents: &[MutationIntent],
) -> Result<MaterializationPlan, MutationError> {
    let delta = finalize_delta(layout, image, context, intents)?;
    let seed = context.repository_seed(intents)?;
    MaterializationPlan::new(
        image,
        &seed,
        &MaterializationIntent::SemanticMutation,
        delta,
    )
    .map_err(Into::into)
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
    let mut index_creations: Vec<String> = Vec::new();
    let mut index_deletions: Vec<String> = Vec::new();

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
                owner: OWNER.to_string(),
                expected: ExpectedPreimage::Absent,
            });
        }
    }

    // Pass 1: issue creations, in canonical request order, get identifiers first.
    for intent in intents {
        if let MutationIntent::CreateIssue { draft } = intent {
            let id = context.allocate();
            let now = context.timestamp();
            let mut issue = (**draft).clone();
            issue.id = id.clone();
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
                tag: "issue_created".to_string(),
                primary: id.clone(),
                secondary: String::new(),
                ordinal: pending_events.len(),
                event: Event::IssueCreated {
                    id: String::new(),
                    issue_id: id.clone(),
                    timestamp: sentinel_time(),
                    title: issue.title.clone(),
                    priority: issue.priority,
                },
            });
        }
    }

    for intent in intents {
        if let MutationIntent::CreateIssueBatch {
            drafts,
            dependencies,
        } = intent
        {
            let ids = (0..drafts.len())
                .map(|_| context.allocate())
                .collect::<Vec<_>>();
            for (index, draft) in drafts.iter().enumerate() {
                let now = context.timestamp();
                let mut issue = draft.clone();
                issue.id = ids[index].clone();
                issue.dependencies = dependencies
                    .iter()
                    .filter(|(dependent, _)| *dependent == index)
                    .map(|(_, dependency)| ids[*dependency].clone())
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
                    tag: "issue_created".to_string(),
                    primary: issue.id.clone(),
                    secondary: String::new(),
                    ordinal: pending_events.len(),
                    event: Event::draft_issue_created(&issue),
                });
                if !issue.dependencies.is_empty() {
                    pending_events.push(PendingEvent {
                        phase: 1,
                        tag: "issue_updated".to_string(),
                        primary: issue.id.clone(),
                        secondary: String::new(),
                        ordinal: pending_events.len(),
                        event: Event::draft_issue_updated(
                            issue.id.clone(),
                            "batch-create".to_string(),
                            vec!["dependencies".to_string(), "state".to_string()],
                        ),
                    });
                }
            }
        }
    }

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
                    tag: "issue_claimed".to_string(),
                    primary: issue_id.clone(),
                    secondary: agent.to_string(),
                    ordinal: pending_events.len(),
                    event: Event::IssueClaimed {
                        id: String::new(),
                        issue_id: issue_id.clone(),
                        timestamp: sentinel_time(),
                        assignee: agent.clone(),
                    },
                });
            }
        }
    }

    // Pass 2b: full-issue semantic updates of existing issues. No identifier is
    // allocated (the id must match a captured issue), preserving the frozen
    // allocation order. `updated_at` is stamped from the single mutation clock;
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
                let records_evidence = intents.iter().any(|intent| match intent {
                    MutationIntent::RecordEvent { event, .. } => match (&**event, state.status) {
                        (
                            Event::GatePassed {
                                issue_id, gate_key, ..
                            },
                            GateStatus::Passed,
                        )
                        | (
                            Event::GateFailed {
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
            actions.push(write_issue_action(image, &updated)?);
        }
    }

    // Pass 2c: an explicitly typed lifecycle-history repair is the sole path
    // that may persist reconstructed historical timestamps. Ordinary updates
    // above continue to preserve those fields from the captured preimage.
    for intent in intents {
        if let MutationIntent::RepairIssueLifecycle { issue } = intent {
            let preimage = captured_issue(image, &issue.id)?
                .ok_or_else(|| MutationError::MissingIssue(issue.id.clone()))?;
            let mut repaired = preimage;
            repaired.first_ready_at = issue.first_ready_at;
            repaired.claimed_at = issue.claimed_at;
            repaired.done_at = issue.done_at;
            actions.push(write_issue_action(image, &repaired)?);
        }
    }

    // Pass 2d: issue deletions remove the captured record and move membership in
    // the same transaction. Absence is an error, matching the old delete command
    // contract without turning this into an idempotent or implicit operation.
    for intent in intents {
        if let MutationIntent::DeleteIssue { issue_id } = intent {
            captured_issue(image, issue_id)?
                .ok_or_else(|| MutationError::MissingIssue(issue_id.clone()))?;
            let path = issue_path(issue_id)?;
            actions.push(RepositoryAction::DeleteFile {
                path: path.clone(),
                owner: OWNER.to_string(),
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
                    bytes: current_bytes,
                    mode: FileMode::Regular,
                    ..
                } if current_bytes == &bytes
            ) {
                actions.push(RepositoryAction::WriteFile {
                    path,
                    owner: OWNER.to_string(),
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
            if crate::gate_presets::BuiltinPresets::load()
                .map_err(|error| MutationError::InvalidGatePreset(error.to_string()))?
                .contains_key(&preset.name)
            {
                return Err(MutationError::GatePresetCollidesWithBuiltin(
                    preset.name.clone(),
                ));
            }
            for parent in ["config", "config/gate-presets"] {
                let path = VirtualPath::data(parent)?;
                match image.entry(&path)? {
                    crate::repository_state::RepositoryEntry::Absent => {
                        actions.push(RepositoryAction::CreateDirectory {
                            path,
                            owner: OWNER.to_string(),
                            expected: ExpectedPreimage::Absent,
                        });
                    }
                    crate::repository_state::RepositoryEntry::Directory { .. } => {}
                    _ => {
                        return Err(MutationError::GatePresetParentNotDirectory(
                            parent.to_string(),
                        ))
                    }
                }
            }
            let path = VirtualPath::data(format!("config/gate-presets/{}.json", preset.name))?;
            if !matches!(
                image.entry(&path)?,
                crate::repository_state::RepositoryEntry::Absent
            ) {
                return Err(MutationError::GatePresetAlreadyExists(preset.name.clone()));
            }
            actions.push(RepositoryAction::WriteFile {
                path,
                owner: OWNER.to_string(),
                expected: ExpectedPreimage::Absent,
                bytes: canonical_json(&**preset, true)?,
                mode: FileMode::Regular,
            });
        }
    }

    // Pass 3: gate-run / other transaction-visible records get identifiers next,
    // in (target, key) order — here keyed by (issue id, gate key).
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
    if !gate_runs.is_empty() {
        let parent = VirtualPath::GATE_RUNS;
        if matches!(
            image.entry(&parent)?,
            crate::repository_state::RepositoryEntry::Absent
        ) {
            actions.push(RepositoryAction::CreateDirectory {
                path: parent,
                owner: OWNER.to_string(),
                expected: ExpectedPreimage::Absent,
            });
        }
    }
    for draft in gate_runs {
        let run_id = context.allocate();
        let mut result = draft.clone();
        result.run_id = run_id.clone();
        let result_path = gate_run_result_relative_path(&run_id)?;
        let dir = VirtualPath::data(result_path.as_path().parent().ok_or_else(|| {
            RepositoryLayoutError::LexicalEscape(result_path.as_path().display().to_string())
        })?)?;
        let file = VirtualPath::data(result_path.as_path())?;
        // The run directory is freshly named by an allocated id, so it is absent.
        actions.push(RepositoryAction::CreateDirectory {
            path: dir,
            owner: OWNER.to_string(),
            expected: ExpectedPreimage::Absent,
        });
        actions.push(RepositoryAction::WriteFile {
            path: file,
            owner: OWNER.to_string(),
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
                tag: event.get_type().to_string(),
                primary,
                secondary,
                ordinal: pending_events.len(),
                event: (**event).clone(),
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

/// Compose the audit-log append action for a set of pending events over the
/// captured `events.jsonl` prefix: order them canonically, assign identifiers (via
/// the context, in the caller's frozen allocation order), stamp the single mutation
/// timestamp, certify a torn tail (a `ProfileApplied` marker is required, and the
/// certifier is composed immediately after the torn partial), and produce the exact
/// bytes. Returns `None` when there are no events. This is the event pass of
/// [`finalize`], factored out so [`finalize_audit_append`] can reuse the one
/// torn-tail authority.
fn compose_event_action(
    image: &RepositoryImage,
    context: &MutationContext,
    mut pending_events: Vec<PendingEvent>,
) -> Result<Option<RepositoryAction>, MutationError> {
    if pending_events.is_empty() {
        return Ok(None);
    }
    let path = VirtualPath::EVENTS;
    let prefix = captured_file_bytes(image, &path)?.unwrap_or(&[]).to_vec();
    // The finalizer owns torn-tail evidence. The reader (parse_known_events) accepts
    // a malformed, unterminated partial line only when the line IMMEDIATELY following
    // it is a `ProfileApplied { isolated_torn_tail: true }` certifier. Detect this
    // before sampling time or allocating identifiers.
    let torn_tail = prefix_has_torn_tail(&prefix);
    let has_marker = pending_events
        .iter()
        .any(|pending| matches!(pending.event, Event::ProfileApplied { .. }));
    if torn_tail && !has_marker {
        // Appending non-certifying events after an uncertified torn tail would
        // produce a log the reader rejects; refuse instead of corrupting it.
        return Err(MutationError::UncertifiedTornTail);
    }

    let now = context.timestamp();
    // Assign identifiers in canonical event order, stamping the single mutation
    // timestamp and the torn-tail evidence.
    pending_events.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
    for pending in &mut pending_events {
        let id = context.allocate();
        pending.event.assign_identity(id, now);
        if let Event::ProfileApplied {
            isolated_torn_tail, ..
        } = &mut pending.event
        {
            *isolated_torn_tail = torn_tail;
        }
    }
    // Over a torn tail, the certifying marker must be composed as the line
    // immediately following the torn partial; the remaining events keep their
    // canonical order after it.
    if torn_tail {
        let marker = pending_events
            .iter()
            .position(|pending| matches!(pending.event, Event::ProfileApplied { .. }))
            .expect("a markerless torn tail was already rejected");
        let certifier = pending_events.remove(marker);
        pending_events.insert(0, certifier);
    }
    let lines = pending_events
        .iter()
        .map(|pending| serialize_event(&pending.event))
        .collect::<Result<Vec<_>, _>>()?;
    let bytes = compose_events(&prefix, &lines);
    Ok(Some(RepositoryAction::WriteFile {
        path: path.clone(),
        owner: OWNER.to_string(),
        expected: expected_of(image, &path)?,
        bytes,
        mode: FileMode::Regular,
    }))
}

/// Compose the exact `events.jsonl` append action for repository-owned provenance
/// events emitted by a delta the record finalizer does not otherwise build — the
/// init/profile finalizers, whose deltas also carry asset/registry/record file
/// actions. Each `(phase, event)` pair is an inline event variant carrying a
/// sentinel id/timestamp; this resets the context's frozen allocation order, then
/// assigns identifiers and the single mutation timestamp and certifies a torn tail
/// exactly as [`finalize`] does — so a `ProfileApplied` line is the authoritative
/// audit record, never a command-computed image. Returns `None` when `events` is
/// empty (no append, no identity or time sampling).
///
/// The context must be the operation's single reused [`MutationContext`] and must
/// not also drive a full [`finalize`] in the same operation (both reset the frozen
/// order); the init/profile paths use only this entry.
/// Build the inline `ProfileApplied` audit event for the finalizer path: a bare
/// variant with an empty id and the sentinel timestamp that [`finalize_audit_append`]
/// overwrites via [`Event::assign_identity`], and a `false` torn-tail flag the
/// finalizer sets from the captured prefix. Unlike `Event::draft_profile_applied` it
/// samples neither a UUID nor the wall clock, so the finalizer — not command code —
/// owns the event's identity, time, and torn-tail evidence.
pub(crate) fn profile_applied_event(
    profile_id: String,
    version: String,
    origin: crate::domain::ProfileOrigin,
    package_hash: String,
    target_hashes: BTreeMap<String, String>,
) -> Event {
    Event::ProfileApplied {
        id: String::new(),
        timestamp: sentinel_time(),
        profile_id,
        version,
        origin,
        package_hash,
        target_hashes,
        isolated_torn_tail: false,
    }
}

pub(crate) fn finalize_audit_append(
    image: &RepositoryImage,
    context: &MutationContext,
    events: Vec<(u8, Event)>,
) -> Result<Option<RepositoryAction>, MutationError> {
    if events.is_empty() {
        return Ok(None);
    }
    context.begin();
    let pending = events
        .into_iter()
        .enumerate()
        .map(|(ordinal, (phase, event))| {
            let (primary, secondary) = event_identities(&event);
            PendingEvent {
                phase,
                tag: event.get_type().to_string(),
                primary,
                secondary,
                ordinal,
                event,
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
    let path = issue_path(&issue.id)?;
    Ok(RepositoryAction::WriteFile {
        path: path.clone(),
        owner: OWNER.to_string(),
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
    created_ids: &[String],
    deleted_ids: &[String],
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
        owner: OWNER.to_string(),
        expected,
        bytes: index.to_pretty_bytes()?,
        mode: FileMode::Regular,
    })
}

/// Primary/secondary ordering identities for an event.
fn event_identities(event: &Event) -> (String, String) {
    let issue_id = event.get_issue_id().to_string();
    match event {
        Event::GatePassed { gate_key, .. }
        | Event::GateFailed { gate_key, .. }
        | Event::GateAdded { gate_key, .. }
        | Event::GateRemoved { gate_key, .. }
        | Event::GateDefinitionCreated { gate_key, .. }
        | Event::GateDefinitionUpdated { gate_key, .. }
        | Event::GateDefinitionRemoved { gate_key, .. } => (issue_id, gate_key.clone()),
        Event::IssueClaimed { assignee, .. } => (issue_id, assignee.to_string()),
        _ => (issue_id, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{GateState, GateStatus};
    use crate::repository_state::{
        CaptureBudget, CaptureSpec, EntryIdentity, RepositoryEntry, RepositoryImage,
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

    fn budget() -> CaptureBudget {
        CaptureBudget {
            max_paths: 16,
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
        let spec = CaptureSpec::phase_one(paths, budget()).unwrap();
        RepositoryImage::close(
            layout(),
            spec,
            entries.into_iter().collect::<BTreeMap<_, _>>(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    }

    fn file_entry(bytes: &[u8]) -> RepositoryEntry {
        RepositoryEntry::File {
            identity: EntryIdentity::for_bytes("obj", bytes).unwrap(),
            bytes: bytes.to_vec(),
            mode: FileMode::Regular,
        }
    }

    fn seeded_issue(id: &str, assignee: Option<Assignee>) -> Issue {
        let mut issue = crate::domain::types::fixture_issue("Title".into(), "Body".into());
        issue.id = id.to_string();
        issue.assignee = assignee;
        issue.created_at = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        issue.updated_at = issue.created_at;
        issue
    }

    fn map_order_draft(reverse: bool) -> Issue {
        let mut issue = crate::domain::types::fixture_issue("Canonical".into(), "Body".into());
        issue.id = "11111111-1111-4111-8111-111111111111".into();
        let context_entries = [("alpha", "one"), ("zeta", "two")];
        let gate_entries = [
            (
                "alpha-gate",
                GateState {
                    status: GateStatus::Passed,
                    updated_by: Some("agent:alpha".parse().unwrap()),
                    updated_at: fixed_instant(),
                },
            ),
            (
                "zeta-gate",
                GateState {
                    status: GateStatus::Failed,
                    updated_by: Some("agent:zeta".parse().unwrap()),
                    updated_at: fixed_instant(),
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
        let created_id = IdAuthority::from_seed([7u8; 32]).uuid_at(0);
        let image = image_with(vec![
            (
                VirtualPath::data("issues").unwrap(),
                RepositoryEntry::Absent,
            ),
            (VirtualPath::INDEX, file_entry(&index_bytes)),
            (VirtualPath::EVENTS, RepositoryEntry::Absent),
            (issue_path(&created_id).unwrap(), RepositoryEntry::Absent),
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
        assert_eq!(
            first.hash(),
            retry.hash(),
            "semantic plan hashes must be stable across retries"
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
        let created_id = IdAuthority::from_seed([7u8; 32]).uuid_at(0);
        let image = image_with(vec![
            (
                VirtualPath::data("issues").unwrap(),
                RepositoryEntry::Absent,
            ),
            (VirtualPath::INDEX, file_entry(&index_bytes)),
            (VirtualPath::EVENTS, RepositoryEntry::Absent),
            (issue_path(&created_id).unwrap(), RepositoryEntry::Absent),
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
                    if path == &issue_path(&created_id).unwrap() =>
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

        let event = Event::GateDefinitionCreated {
            id: "event-1".into(),
            timestamp: fixed_instant(),
            gate_key: "cargo-ci".into(),
        };
        let event_bytes = serialize_event(&event).unwrap();
        assert!(event_bytes.starts_with(b"{"));
        assert!(!event_bytes.contains(&b'\n'));
        assert!(!event_bytes.ends_with(b"\n"));
        let round_trip: Event = serde_json::from_slice(&event_bytes).unwrap();
        assert_eq!(round_trip, event);
    }

    #[test]
    fn test_update_issue_stamps_changed_gate_state_from_mutation_context() {
        let id = "11111111-1111-4111-8111-111111111111";
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
            },
        );
        let bytes = serialize_issue(&preimage).unwrap();
        let image = image_with(vec![(issue_path(id).unwrap(), file_entry(&bytes))]);
        let intent = MutationIntent::UpdateIssue {
            issue: Box::new(updated),
        };
        let first = finalize(&layout(), &image, &ctx(), std::slice::from_ref(&intent)).unwrap();
        let retry = finalize(&layout(), &image, &ctx(), &[intent]).unwrap();
        assert_eq!(first.hash(), retry.hash());
        assert_eq!(first.delta(), retry.delta());
        let written = first
            .delta()
            .actions()
            .iter()
            .find_map(|action| match action {
                RepositoryAction::WriteFile { path, bytes, .. }
                    if path == &issue_path(id).unwrap() =>
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
        let issue = seeded_issue("11111111-1111-4111-8111-111111111111", Some(agent.clone()));
        let issue_bytes = serialize_issue(&issue).unwrap();
        let event = Event::IssueClaimed {
            id: "e".into(),
            issue_id: issue.id.clone(),
            timestamp: fixed_instant(),
            assignee: agent.clone(),
        };
        let events_bytes = compose_events(&[], &[serialize_event(&event).unwrap()]);
        let image = image_with(vec![
            (issue_path(&issue.id).unwrap(), file_entry(&issue_bytes)),
            (VirtualPath::EVENTS, file_entry(&events_bytes)),
        ]);
        // A clock that panics on use proves the no-op samples no time.
        struct PanicClock;
        impl MutationClock for PanicClock {
            fn now(&self) -> DateTime<Utc> {
                panic!("no-op must not sample the mutation clock");
            }
        }
        let context = MutationContext::new(IdAuthority::from_seed([9u8; 32]), Box::new(PanicClock));
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
        let issue = seeded_issue("44444444-4444-4444-8444-444444444444", Some(agent.clone()));
        let issue_bytes = serialize_issue(&issue).unwrap();
        let event = Event::IssueClaimed {
            id: "e".into(),
            issue_id: issue.id.clone(),
            timestamp: fixed_instant(),
            assignee: agent.clone(),
        };
        let events_bytes = compose_events(&[], &[serialize_event(&event).unwrap()]);
        let image = image_with(vec![
            (issue_path(&issue.id).unwrap(), file_entry(&issue_bytes)),
            (VirtualPath::EVENTS, file_entry(&events_bytes)),
        ]);
        struct PanicClock;
        impl MutationClock for PanicClock {
            fn now(&self) -> DateTime<Utc> {
                panic!("no-op must not sample the mutation clock");
            }
        }
        let context = MutationContext::new(IdAuthority::random(), Box::new(PanicClock));
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
        let issue = seeded_issue("22222222-2222-4222-8222-222222222222", Some(agent.clone()));
        let issue_bytes = serialize_issue(&issue).unwrap();
        let image = image_with(vec![
            (issue_path(&issue.id).unwrap(), file_entry(&issue_bytes)),
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
        let issue = seeded_issue("33333333-3333-4333-8333-333333333333", None);
        let issue_bytes = serialize_issue(&issue).unwrap();
        let image = image_with(vec![
            (issue_path(&issue.id).unwrap(), file_entry(&issue_bytes)),
            (VirtualPath::EVENTS, RepositoryEntry::Absent),
        ]);
        let delta = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::ClaimIssue {
                issue_id: issue.id.clone(),
                agent: agent.clone(),
            }],
        )
        .unwrap();
        // Two actions: the assigned issue and the appended claim event.
        assert_eq!(delta.delta().actions().len(), 2);
        // The issue write carries the assignee and a stamped claimed_at.
        let issue_action = delta
            .delta()
            .actions()
            .iter()
            .find(|action| action.path() == &issue_path(&issue.id).unwrap())
            .unwrap();
        let RepositoryAction::WriteFile { bytes, .. } = issue_action else {
            panic!("expected issue write");
        };
        let written: Issue = serde_json::from_slice(bytes).unwrap();
        assert_eq!(written.assignee, Some(agent));
        assert_eq!(written.claimed_at, Some(fixed_instant()));
        assert_eq!(written.updated_at, fixed_instant());
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
            let mut issue = crate::domain::types::fixture_issue("New".into(), "Body".into());
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
            // The new id is deterministic; precompute it to seed its absent slot.
            (
                issue_path(&IdAuthority::from_seed([7u8; 32]).uuid_at(0)).unwrap(),
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
        let expected_id = IdAuthority::from_seed([7u8; 32]).uuid_at(0);
        let issue_action = delta
            .delta()
            .actions()
            .iter()
            .find(|action| action.path() == &issue_path(&expected_id).unwrap())
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
        let issue = seeded_issue("11111111-1111-4111-8111-111111111111", None);
        let issue_bytes = serialize_issue(&issue).unwrap();
        let index_bytes = canonical_json(
            &RepositoryIndex {
                schema_version: 2,
                all_ids: vec![issue.id.clone(), "other".to_string()],
                deleted_ids: Vec::new(),
            },
            true,
        )
        .unwrap();
        let image = image_with(vec![
            (issue_path(&issue.id).unwrap(), file_entry(&issue_bytes)),
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
            RepositoryAction::DeleteFile { path, .. } if path == &issue_path(&issue.id).unwrap()
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
        assert_eq!(index.all_ids, ["other"]);
        assert_eq!(index.deleted_ids, [issue.id]);
    }

    #[test]
    fn test_create_issue_rejects_future_index_without_a_plan() {
        let draft = crate::domain::types::fixture_issue("New".into(), "Body".into());
        let created_id = IdAuthority::from_seed([7u8; 32]).uuid_at(0);
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
            (issue_path(&created_id).unwrap(), RepositoryEntry::Absent),
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
        let issue = seeded_issue("11111111-1111-4111-8111-111111111111", None);
        let issue_bytes = serialize_issue(&issue).unwrap();
        let index_bytes = serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "all_ids": [issue.id],
            "deleted_ids": ["deleted", "deleted"]
        }))
        .unwrap();
        let image = image_with(vec![
            (issue_path(&issue.id).unwrap(), file_entry(&issue_bytes)),
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
        let issue = seeded_issue("22222222-2222-4222-8222-222222222222", None);
        let created_at = issue.created_at;
        let mut updated = issue.clone();
        updated.state = State::Done;
        updated.created_at = DateTime::UNIX_EPOCH;
        updated.done_at = Some(DateTime::UNIX_EPOCH);
        let bytes = serialize_issue(&issue).unwrap();
        let image = image_with(vec![(issue_path(&issue.id).unwrap(), file_entry(&bytes))]);

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
            .find(|action| action.path() == &issue_path(&issue.id).unwrap())
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
    fn test_repair_issue_lifecycle_cannot_mutate_non_lifecycle_fields() {
        let mut preimage = seeded_issue("33333333-3333-4333-8333-333333333333", None);
        preimage.first_ready_at = None;
        preimage.claimed_at = None;
        preimage.done_at = None;
        let bytes = serialize_issue(&preimage).unwrap();
        let image = image_with(vec![(
            issue_path(&preimage.id).unwrap(),
            file_entry(&bytes),
        )]);

        let lifecycle_time = DateTime::parse_from_rfc3339("2021-02-03T04:05:06Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut proposed = preimage.clone();
        proposed.title = "must not persist".into();
        proposed.description = "must not persist".into();
        proposed.state = State::Done;
        proposed.dependencies = vec!["other".into()];
        proposed.labels = vec!["type:epic".into()];
        proposed.first_ready_at = Some(lifecycle_time);
        proposed.claimed_at = Some(lifecycle_time);
        proposed.done_at = Some(lifecycle_time);

        let plan = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::RepairIssueLifecycle {
                issue: Box::new(proposed),
            }],
        )
        .unwrap();
        let action = plan
            .delta()
            .actions()
            .iter()
            .find(|action| action.path() == &issue_path(&preimage.id).unwrap())
            .unwrap();
        let RepositoryAction::WriteFile { bytes, .. } = action else {
            panic!("expected issue write");
        };
        let persisted: Issue = serde_json::from_slice(bytes).unwrap();
        let mut expected = preimage;
        expected.first_ready_at = Some(lifecycle_time);
        expected.claimed_at = Some(lifecycle_time);
        expected.done_at = Some(lifecycle_time);
        assert_eq!(persisted, expected);
    }

    #[test]
    fn test_torn_tail_detection() {
        assert!(!prefix_has_torn_tail(b""));
        assert!(!prefix_has_torn_tail(b"{\"type\":\"x\"}\n"));
        assert!(prefix_has_torn_tail(b"{\"type\":\"x\"}\n{\"partial"));
        assert!(!prefix_has_torn_tail(b"{\"a\":1}"));
    }

    #[test]
    fn test_compose_events_inserts_single_separator() {
        assert_eq!(compose_events(b"", &[b"line".to_vec()]), b"line\n");
        assert_eq!(
            compose_events(b"prev\n", &[b"line".to_vec()]),
            b"prev\nline\n"
        );
        // A non-empty prefix lacking a newline gets exactly one separator.
        assert_eq!(
            compose_events(b"prev", &[b"line".to_vec()]),
            b"prev\nline\n"
        );
    }

    fn profile_applied_marker() -> Event {
        use crate::domain::ProfileOrigin;
        Event::ProfileApplied {
            id: String::new(),
            timestamp: sentinel_time(),
            profile_id: "example".into(),
            version: "1.0".into(),
            origin: ProfileOrigin::Embedded,
            package_hash: "hash".into(),
            target_hashes: BTreeMap::new(),
            isolated_torn_tail: false,
        }
    }

    /// A captured events.jsonl whose final line is a malformed, unterminated tail.
    fn torn_tail_image() -> RepositoryImage {
        let torn = b"{\"type\":\"issue_completed\",\"id\":\"e0\",\"issue_id\":\"i\",\"timestamp\":\"2020-01-01T00:00:00Z\"}\n{\"parti".to_vec();
        image_with(vec![(VirtualPath::EVENTS, file_entry(&torn))])
    }

    #[test]
    fn test_finalize_rejects_markerless_append_over_torn_tail() {
        // A batch with no ProfileApplied certifier appended after an uncertified
        // torn tail would produce a log the reader rejects, so finalize refuses and
        // composes nothing.
        let image = torn_tail_image();
        let event = Event::GateDefinitionCreated {
            id: String::new(),
            timestamp: sentinel_time(),
            gate_key: "cargo-ci".into(),
        };
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

    #[test]
    fn test_finalize_certifier_over_torn_tail_round_trips_through_reader() {
        // A batch containing a ProfileApplied marker composes it immediately after
        // the torn partial line, so the reader accepts the whole log.
        let image = torn_tail_image();
        let delta = finalize(
            &layout(),
            &image,
            &ctx(),
            &[MutationIntent::RecordEvent {
                phase: 0,
                event: Box::new(profile_applied_marker()),
            }],
        )
        .unwrap();
        let action = delta
            .delta()
            .actions()
            .iter()
            .find(|action| action.path() == &VirtualPath::EVENTS)
            .expect("events.jsonl write present");
        let RepositoryAction::WriteFile { bytes, .. } = action else {
            panic!("expected events write");
        };
        let text = std::str::from_utf8(bytes).unwrap();
        // The certifier record immediately follows the torn partial line; JSON
        // object key order is not part of the event-log contract.
        let certifier_follows_tail = text.lines().collect::<Vec<_>>().windows(2).any(|lines| {
            lines[0] == "{\"parti"
                && serde_json::from_str::<Event>(lines[1]).is_ok_and(|event| {
                    matches!(
                        event,
                        Event::ProfileApplied {
                            isolated_torn_tail: true,
                            ..
                        }
                    )
                })
        });
        assert!(certifier_follows_tail);
        // The reader accepts the composed log and recovers the marker.
        let events = crate::domain::parse_known_events(text).unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            Event::ProfileApplied {
                isolated_torn_tail: true,
                ..
            }
        )));
    }
}
