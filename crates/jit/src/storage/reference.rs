//! Projection of the on-disk record layout into a committed markdown reference.
//!
//! Storage facts an adopter has to know are defined in code, not in prose:
//! identifier shapes, the event log, and the issue, gate-run, verdict, and
//! archive-evidence records. [`render_reference_markdown`] projects them into
//! that page, so it is generated from the definitions the binary writes with
//! (`@/inv/single-source-prose`).
//!
//! Every claim on the page is derived rather than restated:
//!
//! - The identifier widths come from the constants that enforce them:
//!   [`SHORT_ID_LENGTH`], the width [`Issue::short_id`](crate::domain::Issue::short_id)
//!   truncates to, and [`IssueIdPrefix::MIN_ID_PREFIX_LENGTH`], the minimum
//!   [`IssueIdPrefix`] accepts.
//! - The event-log sample lines are [`EventTag::sample`] records encoded by
//!   [`serialize_event`] with the newline terminator the finalizer's positioned
//!   append adds to each line (`compose_events`). The tag vocabulary is NOT restated here: it is the event
//!   catalog's own projection ([`crate::domain::event_catalog::event_catalog`]), which this page
//!   links to.
//! - The gate-run path comes from
//!   [`VirtualPath::gate_run_record`](crate::repository_state::VirtualPath::gate_run_record),
//!   shared by the repository-state finalizer and storage readers; the record's
//!   example is a [`GateRunResult`] encoded by [`serialize_gate_run`]; each field's presence when unset is
//!   read off that encoding.
//! - Verdict and archive-evidence paths come from their `VirtualPath`
//!   constructors, and their examples use the same serializers as their
//!   finalizers. Their field rows are checked against schemas derived from their
//!   record types and their serialized keys.
//! - [`GateRunField`] names the record's fields. Conformance tests hold it against
//!   the property list schemars derives from [`GateRunResult`] itself and against
//!   the serialized record, so a field added to the record fails the suite until
//!   the page documents it, and a golden test asserts the committed page equals
//!   the projection.

use crate::declarations::GateStage;
use crate::domain::artifact_execution::ArchiveEvidenceRecord;
use crate::domain::artifact_plan::PlanTarget;
use crate::domain::event_catalog::EventTag;
use crate::domain::gate_findings::{GateFinding, GateFindings};
use crate::domain::identity::{GateKey, GateRunId, IssueId, IssueIdPrefix};
use crate::domain::repository_inputs::InputsDigest;
use crate::domain::{
    GateRunResult, GateRunStatus, GateVerdictRecord, GATE_RUN_SCHEMA_VERSION, SHORT_ID_LENGTH,
};
use crate::repository_state::{
    canonical_json, serialize_event, serialize_gate_run, serialize_gate_verdict, VirtualPath,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::path::Path;

pub(crate) mod test_support {
    /// Repo-relative path of the committed reference this module projects.
    pub const REFERENCE_PATH: &str = "docs/reference/storage-records.md";

    /// The command that renders [`REFERENCE_PATH`] from these definitions, named in
    /// the conformance test's message so a stale reference carries its own repair.
    pub const REFERENCE_GENERATOR: &str = "./scripts/generate-storage-records-reference.sh";
}

/// The page states that a short id is always long enough to hand back as an id
/// prefix. That holds only while the short id is at least as wide as the
/// resolvable minimum, so the claim is checked at compile time against the two
/// constants it is drawn from.
const _: () = assert!(SHORT_ID_LENGTH >= IssueIdPrefix::MIN_ID_PREFIX_LENGTH);

/// The fields of a recorded gate run: one variant per [`GateRunResult`] field.
///
/// [`GateRunField::as_str`] is the JSON key serde writes for the field.
/// Conformance tests hold this list against the schema schemars derives from
/// [`GateRunResult`], so the record and the projection cannot diverge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GateRunField {
    /// `schema_version`
    SchemaVersion,
    /// `run_id`
    RunId,
    /// `gate_key`
    GateKey,
    /// `stage`
    Stage,
    /// `issue_id`
    IssueId,
    /// `commit`
    Commit,
    /// `branch`
    Branch,
    /// `status`
    Status,
    /// `started_at`
    StartedAt,
    /// `completed_at`
    CompletedAt,
    /// `duration_ms`
    DurationMs,
    /// `exit_code`
    ExitCode,
    /// `stdout`
    Stdout,
    /// `stderr`
    Stderr,
    /// `command`
    Command,
    /// `by`
    By,
    /// `message`
    Message,
    /// `findings`
    Findings,
    /// `inputs_digest`
    InputsDigest,
    /// `origin`
    Origin,
    /// `evaluation_stability`
    EvaluationStability,
}

/// `tokens` as code spans joined into an English alternative: `a` or `b`, or
/// `a`, `b`, or `c`.
fn code_alternatives(tokens: &[&str]) -> String {
    let spans = tokens
        .iter()
        .map(|token| format!("`{token}`"))
        .collect::<Vec<_>>();
    match spans.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [first, second] => format!("{first} or {second}"),
        [leading @ .., last] => format!("{}, or {last}", leading.join(", ")),
    }
}

impl GateRunField {
    /// Every field of the record, in [`GateRunResult`] declaration order.
    pub const ALL: [GateRunField; 21] = [
        GateRunField::SchemaVersion,
        GateRunField::RunId,
        GateRunField::GateKey,
        GateRunField::Stage,
        GateRunField::IssueId,
        GateRunField::Commit,
        GateRunField::Branch,
        GateRunField::Status,
        GateRunField::StartedAt,
        GateRunField::CompletedAt,
        GateRunField::DurationMs,
        GateRunField::ExitCode,
        GateRunField::Stdout,
        GateRunField::Stderr,
        GateRunField::Command,
        GateRunField::By,
        GateRunField::Message,
        GateRunField::Findings,
        GateRunField::InputsDigest,
        GateRunField::Origin,
        GateRunField::EvaluationStability,
    ];

    /// The JSON key serde writes for this field.
    pub fn as_str(self) -> &'static str {
        match self {
            GateRunField::SchemaVersion => "schema_version",
            GateRunField::RunId => "run_id",
            GateRunField::GateKey => "gate_key",
            GateRunField::Stage => "stage",
            GateRunField::IssueId => "issue_id",
            GateRunField::Commit => "commit",
            GateRunField::Branch => "branch",
            GateRunField::Status => "status",
            GateRunField::StartedAt => "started_at",
            GateRunField::CompletedAt => "completed_at",
            GateRunField::DurationMs => "duration_ms",
            GateRunField::ExitCode => "exit_code",
            GateRunField::Stdout => "stdout",
            GateRunField::Stderr => "stderr",
            GateRunField::Command => "command",
            GateRunField::By => "by",
            GateRunField::Message => "message",
            GateRunField::Findings => "findings",
            GateRunField::InputsDigest => "inputs_digest",
            GateRunField::Origin => "origin",
            GateRunField::EvaluationStability => "evaluation_stability",
        }
    }

    /// What the field records.
    fn description(self) -> String {
        match self {
            GateRunField::SchemaVersion => format!(
                "Record-format version of this run record; this binary writes \
                 `{GATE_RUN_SCHEMA_VERSION}`. It versions the run record alone, independently \
                 of the repository format version in `index.json`."
            ),
            GateRunField::RunId => {
                "The run's identifier, and the stem of its record's file name. Each execution \
                 mints a fresh UUID v4, so a rerun records a new file instead of overwriting the \
                 previous run."
                    .to_string()
            }
            GateRunField::GateKey => {
                "Key of the gate that ran, as registered in `.jit/gates.toml`.".to_string()
            }
            GateRunField::Stage => format!(
                "Stage the gate ran at: {}.",
                code_alternatives(&GateStage::ALL.map(GateStage::as_str))
            ),
            GateRunField::IssueId => {
                "Full id of the issue the run is about, and the name of the directory under \
                 `gate-runs/` that holds the record."
                    .to_string()
            }
            GateRunField::Commit => {
                "Git commit the checker was launched at, when the working directory is a git \
                 repository. Unset for a run that launched no checker: a verdict taken from an \
                 earlier run ran at no commit of its own, and `origin` names the run that \
                 carries one."
                    .to_string()
            }
            GateRunField::Branch => {
                "Git branch the checker was launched on, under the same conditions as \
                 `commit`."
                    .to_string()
            }
            GateRunField::Status => format!(
                "The run's verdict: {}. For an executed checker the exit status decides it, as \
                 [Checker Exit Codes](../how-to/custom-gates.md#exit-codes) defines.",
                code_alternatives(&GateRunStatus::ALL.map(GateRunStatus::as_str))
            ),
            GateRunField::StartedAt => {
                "RFC 3339 timestamp taken before the checker is launched.".to_string()
            }
            GateRunField::CompletedAt => {
                "RFC 3339 timestamp taken once the checker has returned.".to_string()
            }
            GateRunField::DurationMs => {
                "Wall-clock duration of the checker, in milliseconds.".to_string()
            }
            GateRunField::ExitCode => {
                "Exit code the checker returned; unset when it was killed by a signal or by its \
                 timeout."
                    .to_string()
            }
            GateRunField::Stdout => {
                "The checker's captured standard output, kept verbatim — including any findings \
                 block, which `findings` carries in parsed form."
                    .to_string()
            }
            GateRunField::Stderr => {
                "The checker's captured standard error, kept verbatim.".to_string()
            }
            GateRunField::Command => "The command line that was executed.".to_string(),
            GateRunField::By => "Who triggered the run.".to_string(),
            GateRunField::Message => "Free-text note attached to the run.".to_string(),
            GateRunField::Findings => {
                "Structured findings parsed from the checker's machine-readable block, carrying \
                 the checker's `verdict`, a `summary`, and the `findings` array. Each finding may \
                 carry an optional `references` array of opaque strings; it is omitted when \
                 empty. Unset when the checker emitted no such block; the raw `stdout` is kept \
                 either way. `jit gate status --findings` prints this field."
                    .to_string()
            }
            GateRunField::InputsDigest => {
                "Digest of the repository files the gate declares its checker reads, taken \
                 before the verdict was obtained. Two runs of one gate carrying the same value \
                 read byte-identical content at byte-identical paths. Unset when the gate \
                 declares no inputs, which is what keeps its checker running on every \
                 evaluation."
                    .to_string()
            }
            GateRunField::Origin => {
                "Where the verdict came from. `{\"derivation\": \"executed\"}` means this run \
                 ran the checker; `{\"derivation\": \"reused\", \"source_issue\": \"<issue-id>\", \
                 \"source_run\": \"<run-id>\"}` \
                 means it carries the named earlier run's verdict, whose `inputs_digest` \
                 matched, without the checker running again. The report text behind a reused \
                 verdict lives at the run addressed by `source_issue` and `source_run`. \
                 `{\"derivation\": \"attested\"}` means the \
                 run records a manual gate's attestation: `by` names the attestor and \
                 `started_at` the time."
                    .to_string()
            }
            GateRunField::EvaluationStability => {
                "Whether the evaluation inputs still matched the repository when the run was \
                 recorded. `stable` means they held still; `inputs_moved` means the verdict is \
                 the checker's verdict over the state captured before it started, not over the \
                 state the record was published against. A reader can therefore distinguish \
                 post-run repository drift without treating the completed verdict as discarded."
                    .to_string()
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RecordField {
    name: &'static str,
    meaning: &'static str,
}

const GATE_VERDICT_FIELDS: [RecordField; 2] = [
    RecordField {
        name: "issue_id",
        meaning: "Full id of the issue whose gate-run record holds the reusable verdict.",
    },
    RecordField {
        name: "run_id",
        meaning: "Id of the run record holding the verdict; the reader loads it with `issue_id`.",
    },
];

const ARCHIVE_EVIDENCE_FIELDS: [RecordField; 2] = [
    RecordField {
        name: "target",
        meaning: "Container issue id or normalized repository-relative document path this evidence belongs to.",
    },
    RecordField {
        name: "executions",
        meaning: "Archive execution payloads for this target in order, shared with their archive events.",
    },
];

fn record_field_rows(fields: &[RecordField]) -> String {
    fields
        .iter()
        .map(|field| format!("| `{}` | always | {} |", field.name, cell(field.meaning)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// How a field appears in the stored JSON when it carries no value.
///
/// Read off the sample records this module serializes rather than declared, so it
/// tracks the record's serde attributes instead of mirroring them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presence {
    /// The field always carries a value.
    Always,
    /// The key is written with a `null` value when the field is unset.
    NullWhenUnset,
    /// The key is left out of the object entirely when the field is unset.
    OmittedWhenUnset,
}

impl Presence {
    /// The presence as the reference's table renders it.
    fn as_str(self) -> &'static str {
        match self {
            Presence::Always => "always",
            Presence::NullWhenUnset => "`null` when unset",
            Presence::OmittedWhenUnset => "omitted when unset",
        }
    }
}

/// Run id used in the reference's example record, chosen so the example's path
/// and its `run_id` field are visibly the same string.
fn sample_run_id() -> GateRunId {
    GateRunId::from_uuid(uuid::Uuid::from_u128(
        0x7b2b0a4c_2f5a_4d3e_9b1a_6c8f0d5e4a21,
    ))
}

/// Issue id used in the reference's example record, which its path names too.
fn sample_issue_id() -> IssueId {
    IssueId::from_uuid(uuid::Uuid::from_u128(
        0x9d1f6c02_4a77_4f2b_8f3d_5e0b7a1c8e64,
    ))
}

fn sample_inputs_digest() -> InputsDigest {
    let mut builder =
        crate::domain::repository_inputs::InputsDigestBuilder::new(b"gate declaration");
    builder.push_file("crates/jit/src/lib.rs", b"pub mod domain;\n");
    builder.finish()
}

fn sample_gate_verdict() -> GateVerdictRecord {
    GateVerdictRecord {
        issue_id: sample_issue_id(),
        run_id: sample_run_id(),
    }
}

fn sample_archive_evidence() -> ArchiveEvidenceRecord {
    ArchiveEvidenceRecord {
        target: PlanTarget::Document {
            path: "docs/reference/storage-records.md".to_string(),
        },
        executions: Vec::new(),
    }
}

/// The run record's path as the reference prints it, with the ids as placeholders.
fn documented_result_path() -> String {
    let (issue, run) = (sample_issue_id(), sample_run_id());
    Path::new(".jit")
        .join(
            VirtualPath::gate_run_record(&issue, &run)
                .relative()
                .as_path(),
        )
        .display()
        .to_string()
        .replace(issue.as_str(), "<issue-id>")
        .replace(run.as_str(), "<run-id>")
}

fn data_path(path: &VirtualPath) -> String {
    Path::new(".jit")
        .join(path.relative().as_path())
        .display()
        .to_string()
}

fn documented_gate_verdict_path() -> String {
    let gate_key: GateKey = "tests".parse().expect("sample gate key is valid");
    let digest = sample_inputs_digest();
    data_path(&VirtualPath::gate_verdict_record(&gate_key, &digest))
        .replace(gate_key.as_str(), "<gate-key>")
        .replace(digest.as_str(), "<inputs-digest>")
}

fn documented_archive_evidence_paths() -> [String; 2] {
    let issue_id = sample_issue_id();
    let container_path = data_path(&VirtualPath::archive_evidence_record(
        &PlanTarget::Container {
            id: issue_id.clone(),
        },
    ));
    let container_path = container_path.replace(issue_id.as_str(), "<issue-id>");

    let document_path = data_path(&VirtualPath::archive_evidence_record(
        &PlanTarget::Document {
            path: "docs/reference/storage-records.md".to_string(),
        },
    ));
    let digest = document_path
        .rsplit('/')
        .next()
        .and_then(|filename| filename.strip_suffix(".json"))
        .expect("the document target path ends in a digest filename");
    let document_path = document_path.replace(digest, "<sha256-of-normalized-path>");

    [container_path, document_path]
}

/// Fixed timestamp for the reference's sample records, so the projection is
/// deterministic.
fn sample_timestamp() -> DateTime<Utc> {
    DateTime::from_timestamp(1_768_000_000, 0).unwrap_or_default()
}

/// A gate run with every optional field set: the widest record the encoding can
/// write.
///
/// The struct literal is exhaustive, so a field added to [`GateRunResult`] fails
/// to compile until it is sampled here.
fn sample_gate_run() -> GateRunResult {
    GateRunResult {
        schema_version: GATE_RUN_SCHEMA_VERSION,
        run_id: sample_run_id(),
        gate_key: "tests".parse().unwrap(),
        stage: GateStage::Postcheck,
        issue_id: sample_issue_id(),
        commit: Some("9f1c0f0a2b3c4d5e6f708192a3b4c5d6e7f80910".to_string()),
        branch: Some("main".to_string()),
        status: GateRunStatus::Passed,
        started_at: sample_timestamp(),
        completed_at: Some(sample_timestamp()),
        duration_ms: Some(12_480),
        exit_code: Some(0),
        stdout: "test result: ok. 812 passed; 0 failed".to_string(),
        stderr: String::new(),
        command: "cargo test --workspace".to_string(),
        by: Some("agent:worker-1".parse().unwrap()),
        message: Some("all suites green".to_string()),
        findings: Some(GateFindings {
            verdict: "pass".to_string(),
            summary: "no blocking findings".to_string(),
            findings: vec![GateFinding {
                id: "F1".to_string(),
                severity: "low".to_string(),
                disposition: Some("advisory".to_string()),
                origin: Some("issue-impact".to_string()),
                summary: "the temp path deserves a name".to_string(),
                file: Some("crates/jit/src/storage/json.rs".to_string()),
                line: Some(793),
                references: vec!["@/inv/atomic-writes".to_string()],
            }],
        }),
        // Produced by the same builder the evaluator digests with, so the
        // sample cannot show a value the encoding could not have written.
        inputs_digest: Some(sample_inputs_digest()),
        origin: crate::domain::GateVerdictOrigin::Reused {
            source_issue: crate::domain::identity::IssueId::from_uuid(uuid::Uuid::from_u128(
                0x6a2b8c40_9d15_4f37_8e62_0b3c7d5e1f90,
            )),
            source_run: GateRunId::from_uuid(uuid::Uuid::from_u128(
                0x1c4e8a90_3d2b_4f61_9a07_5b8c2d1e6f34,
            )),
        },
        evaluation_stability: crate::domain::GateEvaluationStability::InputsMoved,
    }
}

/// The same run with every optional field cleared.
///
/// Encoding this next to [`sample_gate_run`] is what tells the projection whether
/// an unset field is written as `null` or left out, so no hand-kept nullability
/// list exists to drift from the record's serde attributes.
fn sample_gate_run_minimal() -> GateRunResult {
    GateRunResult {
        commit: None,
        branch: None,
        completed_at: None,
        duration_ms: None,
        exit_code: None,
        by: None,
        message: None,
        findings: None,
        inputs_digest: None,
        origin: crate::domain::GateVerdictOrigin::Executed,
        evaluation_stability: crate::domain::GateEvaluationStability::Stable,
        ..sample_gate_run()
    }
}

/// Every field paired with its presence, read off the encoding of
/// [`sample_gate_run_minimal`].
///
/// A key the encoder leaves out of the cleared record is omitted when unset; a
/// key it writes as `null` is nullable; anything else always carries a value.
fn field_presence() -> Result<Vec<(GateRunField, Presence)>> {
    let cleared = serde_json::to_value(sample_gate_run_minimal())?;

    Ok(GateRunField::ALL
        .iter()
        .map(|&field| {
            let presence = match cleared.get(field.as_str()) {
                None => Presence::OmittedWhenUnset,
                Some(value) if value.is_null() => Presence::NullWhenUnset,
                Some(_) => Presence::Always,
            };
            (field, presence)
        })
        .collect())
}

/// The event-log sample block: one encoded record per listed tag.
///
/// Each line is [`serialize_event`] of an [`Event`](crate::domain::Event) plus
/// the terminating newline the finalizer appends — so the block is an encoding
/// of real records rather than a transcription of one. The three tags span the
/// scopes an adopter has to tell apart: two issue-scoped records that name their
/// issue, and a registry-scoped record that carries no `issue_id`.
fn event_log_sample() -> Result<String> {
    [
        EventTag::IssueCreated,
        EventTag::IssueStateChanged,
        EventTag::GateDefinitionCreated,
    ]
    .iter()
    .map(|tag| {
        Ok(format!(
            "{}\n",
            String::from_utf8(serialize_event(&tag.sample())?)?
        ))
    })
    .collect()
}

/// Escape the markdown table cell separator so a `|` in a description cannot
/// break the rendered row.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// Render the storage-record reference.
///
/// The returned string is the page's full contents. The identifier widths come
/// from the constants that enforce them, the event-log block from
/// [`EventTag::sample`] records encoded the way the log is written, and the
/// gate-run, verdict, and archive-evidence paths, example records, and field
/// tables from their path constructors and record types — so the page cannot
/// drift from the code that writes and reads the files. The conformance test in
/// this module asserts the committed file equals this output.
///
/// # Errors
///
/// Propagates a `serde_json` failure while encoding one of the sample records.
pub fn render_reference_markdown() -> Result<String> {
    let example_run = String::from_utf8(serialize_gate_run(&sample_gate_run())?)?;
    let example_verdict = String::from_utf8(serialize_gate_verdict(&sample_gate_verdict())?)?;
    let example_archive_evidence =
        String::from_utf8(canonical_json(&sample_archive_evidence(), true)?)?;
    let events = event_log_sample()?;

    // The run's path, straight from the constructor finalization and readers
    // share, so the page names the live layout rather than a prose copy of it.
    let result_path = documented_result_path();
    let verdict_path = documented_gate_verdict_path();
    let [container_evidence_path, document_evidence_path] = documented_archive_evidence_paths();

    let min_prefix_length = IssueIdPrefix::MIN_ID_PREFIX_LENGTH;
    // One line per rule, so the interpolated minimum cannot skew the wrapping.
    let prefix_rules = [
        format!(
            "- the normalized input must be at least {min_prefix_length} characters; below the \
             minimum it is refused as an argument error (exit code 2, see \
             [Exit Codes](exit-codes.md)) before any issue record is read;"
        ),
        "- it names every id whose normalized form it prefixes, so a full id resolves in any \
         case or hyphenation;"
            .to_string(),
        "- it must name exactly one id among the repository's issues. Several matches are refused as \
         ambiguous, and the error lists the candidates."
            .to_string(),
    ]
    .join("\n");

    let fields = field_presence()?
        .iter()
        .map(|(field, presence)| {
            format!(
                "| `{key}` | {presence} | {description} |",
                key = field.as_str(),
                presence = presence.as_str(),
                description = cell(&field.description()),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let verdict_fields = record_field_rows(&GATE_VERDICT_FIELDS);
    let archive_evidence_fields = record_field_rows(&ARCHIVE_EVIDENCE_FIELDS);

    Ok(format!(
        "<!-- Generated from `crate::storage::reference` — do not edit by hand. -->\n\
         \n\
         # Storage Record Layout\n\
         \n\
         > **Diátaxis Type:** Reference\n\
         \n\
         The shape of an issue identifier, of a line in the event log, and of the issue,\n\
         gate-run, verdict, and archive-evidence records.\n\
         This page is generated from the definitions the binary writes with —\n\
         `crates/jit/src/domain/types.rs` and `crates/jit/src/storage/` — so its widths,\n\
         paths, and field lists are the ones in force. For the `.jit/` directory as a\n\
         whole, its configuration and registry files, and the issue record's own field\n\
         table, see [Storage Format](storage-format.md).\n\
         \n\
         ## Issue Identifiers\n\
         \n\
         An issue's `id` is a UUID v4, stored in the canonical hyphenated form\n\
         (`9d1f6c02-4a77-4f2b-8f3d-5e0b7a1c8e64`). Creation mints it, the issue's record\n\
         lives at `.jit/issues/<id>.json`, and every stored cross-reference — a\n\
         `dependencies` entry, an event's `issue_id`, a gate run's `issue_id` — carries\n\
         this full id.\n\
         \n\
         The **short id** the CLI prints is the first {SHORT_ID_LENGTH} characters of that\n\
         string. It is a human-facing convention computed on read, not a stored field: no\n\
         record carries it, and nothing enforces its uniqueness — two issues whose UUIDs\n\
         share their leading {SHORT_ID_LENGTH} characters would print the same short id.\n\
         \n\
         Commands take an id **prefix** wherever they take an issue id. Resolution\n\
         lowercases the input and drops its hyphens, then:\n\
         \n\
         {prefix_rules}\n\
         \n\
         A short id is {SHORT_ID_LENGTH} characters and the minimum is {min_prefix_length},\n\
         so a short id printed by one command is always long enough to hand back to the next.\n\
         \n\
         ## Event Log Records\n\
         \n\
         `.jit/events.jsonl` is the event log, in JSON Lines: one JSON object per line,\n\
         each terminated by a newline. A record is serialized to a single line with its\n\
         object keys sorted, and a mutation positions its append at the log's committed\n\
         length, the end of its last newline-terminated line;\n\
         records are never rewritten in place. The publication contract is\n\
         [Publication and Recovery](storage-format.md#publication-and-recovery). Reading\n\
         parses the file line by line, skipping blank lines. Every record carries a\n\
         `type` tag, its own `id`, and a `timestamp`; the remaining fields are flat on\n\
         the object and vary by tag.\n\
         \n\
         ```jsonl\n\
         {events}\
         ```\n\
         \n\
         The tag vocabulary — every `type` value, what appends it, and which tags carry an\n\
         `issue_id` — is the generated [Event Log Tags](events.md) reference, which this\n\
         page does not restate.\n\
         \n\
         ## Gate Run Records\n\
         \n\
         Running a gate records its result at `{result_path}`.\n\
         The `<run-id>` is a UUID v4 minted per execution, so runs accumulate: a rerun\n\
         writes a new record beside the old one in its issue's directory, and an issue's\n\
         run history is one listing of that directory. The record is written as\n\
         pretty-printed JSON in the same redo record as its coupled issue and event\n\
         updates.\n\
         \n\
         ```json\n\
         {example_run}\n\
         ```\n\
         \n\
         The example above carries every field. The **presence** column below says what\n\
         the encoding does with a field that has no value: `always` fields are written on\n\
         every record, `null when unset` fields keep their key, and `omitted when unset`\n\
         fields drop out of the object entirely — so a reader must treat an absent key and\n\
         a `null` one alike.\n\
         \n\
         | Field | Presence | Meaning |\n\
         | --- | --- | --- |\n\
         {fields}\n\
         \n\
         ## Gate Verdict Records\n\
         \n\
         A reusable verdict is addressed by gate key and the digest of the gate's declared\n\
         inputs at `{verdict_path}`. The digest identifies the content of those declared\n\
         inputs. The repository-state `finalize` mutation writes\n\
         or replaces this record in the same redo record as its gate run, when that run\n\
         executed its checker and passed or failed. A rerun over the same key replaces the\n\
         prior pointer; other verdict keys are independent.\n\
         \n\
         ```json\n\
         {example_verdict}\n\
         ```\n\
         \n\
         `captured_reusable_gate_run` in `commands/gate_check.rs` reads this record by\n\
         `(gate_key, inputs_digest)`, then loads the run addressed by `issue_id` and `run_id`.\n\
         It does not search a run listing.\n\
         \n\
         | Field | Presence | Meaning |\n\
         | --- | --- | --- |\n\
         {verdict_fields}\n\
         \n\
         ## Archive Evidence Records\n\
         \n\
         Archive evidence is addressed by its target. A container record is written at\n\
         `{container_evidence_path}`; a document record is written at\n\
         `{document_evidence_path}`, where the digest is SHA-256 over the normalized\n\
         repository-relative document path. The archive execution finalizer\n\
         (`finalize_archive_execution`) appends the execution to this record in the same\n\
         redo record as the archive event. The archive preview reader,\n\
         `IssueStore::read_archive_evidence_record(target)`, loads it by target; execution\n\
         captures the same path to append prior evidence.\n\
         \n\
         ```json\n\
         {example_archive_evidence}\n\
         ```\n\
         \n\
         | Field | Presence | Meaning |\n\
         | --- | --- | --- |\n\
         {archive_evidence_fields}\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema_for;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use uuid::Uuid;

    /// Absolute path of the committed reference, resolved from the crate root so
    /// the test is independent of the process working directory.
    fn reference_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(test_support::REFERENCE_PATH)
    }

    /// REQ-04 (field completeness): the projected field list must be exactly the
    /// property list schemars derives from `GateRunResult`.
    ///
    /// This is what binds the page to the record type rather than to a mirror of
    /// it: a field added to `GateRunResult` appears in the derived schema, and the
    /// suite fails here until `GateRunField` — and with it the page's table —
    /// documents it.
    #[test]
    fn test_gate_run_fields_match_derived_schema() {
        let schema =
            serde_json::to_value(schema_for!(GateRunResult)).expect("GateRunResult schema is JSON");
        let derived: BTreeSet<String> = schema["properties"]
            .as_object()
            .expect("a struct derives an object schema with properties")
            .keys()
            .cloned()
            .collect();

        let projected: BTreeSet<String> = GateRunField::ALL
            .iter()
            .map(|field| field.as_str().to_string())
            .collect();

        assert_eq!(
            projected, derived,
            "GateRunField must name every GateRunResult field; \
             left = projection, right = derived schema"
        );
    }

    /// REQ-04 (encoding): the projected keys are the keys the record actually
    /// writes — the same `serde_json` encoding the repository-state finalizer
    /// persists.
    #[test]
    fn test_gate_run_fields_match_serialized_record() {
        let record = serde_json::to_value(sample_gate_run()).expect("a gate run serializes");
        let written: BTreeSet<String> = record
            .as_object()
            .expect("a gate run record is a JSON object")
            .keys()
            .cloned()
            .collect();

        let projected: BTreeSet<String> = GateRunField::ALL
            .iter()
            .map(|field| field.as_str().to_string())
            .collect();

        assert_eq!(projected, written);
    }

    /// Every field carries a description, and the table renders one row per field.
    #[test]
    fn test_every_field_is_described_and_rendered() {
        let page = render_reference_markdown().expect("the reference renders");
        for field in GateRunField::ALL {
            assert!(
                !field.description().is_empty(),
                "`{}` has no description",
                field.as_str()
            );
            assert!(
                page.contains(&format!("| `{}` |", field.as_str())),
                "the rendered table is missing the row for `{}`",
                field.as_str()
            );
        }
        for field in GATE_VERDICT_FIELDS.iter().chain(&ARCHIVE_EVIDENCE_FIELDS) {
            assert!(!field.meaning.is_empty());
            assert!(
                page.contains(&format!("| `{}` | always |", field.name)),
                "the rendered table is missing the row for `{}`",
                field.name
            );
        }
    }

    #[test]
    fn test_gate_verdict_fields_match_derived_schema() {
        let schema = serde_json::to_value(schema_for!(GateVerdictRecord))
            .expect("GateVerdictRecord schema is JSON");
        let derived: BTreeSet<String> = schema["properties"]
            .as_object()
            .expect("a struct derives an object schema with properties")
            .keys()
            .cloned()
            .collect();
        let projected: BTreeSet<String> = GATE_VERDICT_FIELDS
            .iter()
            .map(|field| field.name.to_string())
            .collect();

        assert_eq!(projected, derived);
    }

    #[test]
    fn test_gate_verdict_fields_match_serialized_record() {
        let record =
            serde_json::to_value(sample_gate_verdict()).expect("a gate verdict record serializes");
        let written: BTreeSet<String> = record
            .as_object()
            .expect("a gate verdict record is a JSON object")
            .keys()
            .cloned()
            .collect();
        let projected: BTreeSet<String> = GATE_VERDICT_FIELDS
            .iter()
            .map(|field| field.name.to_string())
            .collect();

        assert_eq!(projected, written);
    }

    #[test]
    fn test_archive_evidence_fields_match_derived_schema() {
        let schema = serde_json::to_value(schema_for!(ArchiveEvidenceRecord))
            .expect("ArchiveEvidenceRecord schema is JSON");
        let derived: BTreeSet<String> = schema["properties"]
            .as_object()
            .expect("a struct derives an object schema with properties")
            .keys()
            .cloned()
            .collect();
        let projected: BTreeSet<String> = ARCHIVE_EVIDENCE_FIELDS
            .iter()
            .map(|field| field.name.to_string())
            .collect();

        assert_eq!(projected, derived);
    }

    #[test]
    fn test_archive_evidence_fields_match_serialized_record() {
        let record = serde_json::to_value(sample_archive_evidence())
            .expect("an archive evidence record serializes");
        let written: BTreeSet<String> = record
            .as_object()
            .expect("an archive evidence record is a JSON object")
            .keys()
            .cloned()
            .collect();
        let projected: BTreeSet<String> = ARCHIVE_EVIDENCE_FIELDS
            .iter()
            .map(|field| field.name.to_string())
            .collect();

        assert_eq!(projected, written);
    }

    #[test]
    fn test_reference_documents_verdict_and_archive_evidence_records() {
        let page = render_reference_markdown().expect("the reference renders");

        for required in [
            "## Gate Verdict Records",
            "gate-runs/verdicts/",
            "## Archive Evidence Records",
            "archive-evidence/",
            "`issue_id`",
            "`run_id`",
            "`target`",
            "`executions`",
            "captured_reusable_gate_run",
            "read_archive_evidence_record",
            "finalize_archive_execution",
        ] {
            assert!(page.contains(required), "reference is missing `{required}`");
        }
        let [container_path, document_path] = documented_archive_evidence_paths();
        for path in [
            documented_gate_verdict_path(),
            container_path,
            document_path,
        ] {
            assert!(
                page.contains(&format!("`{path}`")),
                "reference is missing `{path}`"
            );
        }
    }

    /// The presence column reports what the encoding does, not what the table
    /// claims: `commit` keeps a `null` key when unset, `findings` drops out, and a
    /// mandatory field is always written.
    #[test]
    fn test_presence_follows_the_cleared_encoding() {
        let presence = field_presence().expect("the samples encode");
        let of = |wanted: GateRunField| {
            presence
                .iter()
                .find(|(field, _)| *field == wanted)
                .map(|(_, presence)| *presence)
        };
        assert_eq!(of(GateRunField::Commit), Some(Presence::NullWhenUnset));
        assert_eq!(of(GateRunField::Findings), Some(Presence::OmittedWhenUnset));
        assert_eq!(of(GateRunField::RunId), Some(Presence::Always));

        let cleared =
            serde_json::to_value(sample_gate_run_minimal()).expect("the cleared run serializes");
        let cleared = cleared.as_object().expect("a record is a JSON object");
        assert!(!cleared.contains_key(GateRunField::Findings.as_str()));
        assert!(cleared[GateRunField::Commit.as_str()].is_null());
    }

    /// REQ-01: a mutation derives each new issue's id as a UUID v4, and the short
    /// id is the first `SHORT_ID_LENGTH` characters of an id — derived on read,
    /// never a stored field.
    #[test]
    fn test_issue_id_is_uuid_v4_and_short_id_is_its_prefix() {
        let id = crate::repository_state::MutationContext::preview().issue_id_at(0);
        let parsed = Uuid::parse_str(id.as_str()).expect("an issue id is a UUID");
        assert_eq!(parsed.get_version(), Some(uuid::Version::Random));
        assert_eq!(parsed.hyphenated().to_string(), id.as_str());

        let issue = crate::test_utils::fixture_issue("Probe".to_string(), String::new());

        assert_eq!(issue.short_id().len(), SHORT_ID_LENGTH);
        assert!(issue.id.as_str().starts_with(&issue.short_id()));

        // The record stores no short id: the encoded issue carries the full `id`
        // and no key holding its truncation.
        let record = serde_json::to_value(&issue).expect("an issue serializes");
        let record = record
            .as_object()
            .expect("an issue record is a JSON object");
        assert_eq!(record["id"], serde_json::json!(issue.id));
        assert!(!record
            .values()
            .any(|value| value == &serde_json::json!(issue.short_id())));
    }

    /// REQ-03: the page names the path shared by finalization and readers, taken
    /// from the canonical path constructor rather than reconstructed.
    #[test]
    fn test_render_projects_the_storage_result_path() {
        let page = render_reference_markdown().expect("the reference renders");
        assert!(page.contains(&format!("`{}`", documented_result_path())));
    }

    /// REQ-02: the page states the event-log serialization and links the tag
    /// catalog instead of copying it — no tag beyond the three sampled records
    /// appears on the page (`@/inv/single-source-prose`).
    #[test]
    fn test_event_section_links_the_catalog_instead_of_copying_it() {
        let page = render_reference_markdown().expect("the reference renders");
        assert!(page.contains(".jit/events.jsonl"));
        assert!(page.contains("[Event Log Tags](events.md)"));

        let sampled = [
            EventTag::IssueCreated,
            EventTag::IssueStateChanged,
            EventTag::GateDefinitionCreated,
        ];
        for tag in EventTag::ALL {
            if sampled.contains(&tag) {
                assert!(
                    page.contains(&format!("\"type\":\"{}\"", tag.as_str())),
                    "the sample block should carry an encoded `{}` record",
                    tag.as_str()
                );
            } else {
                assert!(
                    !page.contains(tag.as_str()),
                    "`{}` is cataloged in events.md; this page must not restate the vocabulary",
                    tag.as_str()
                );
            }
        }
    }

    /// REQ-04 (projection freshness): the committed reference must equal the
    /// projection. Changing an identifier width, the record's fields, or its
    /// encoding without refreshing the page fails here.
    #[test]
    fn test_committed_reference_matches_projection() {
        let committed = std::fs::read_to_string(reference_path())
            .expect("committed storage-records reference should exist");
        assert_eq!(
            committed,
            render_reference_markdown().expect("the reference renders"),
            "{} is stale — regenerate it from `crate::storage::reference` (run: {})",
            test_support::REFERENCE_PATH,
            test_support::REFERENCE_GENERATOR,
        );
    }
}
