//! Projection of the on-disk record layout into a committed markdown reference.
//!
//! Three storage facts an adopter has to know are defined in code, not in prose:
//! the shape of an issue identifier, the serialization of the event log, and the
//! layout of a recorded gate run. [`render_reference_markdown`] projects them
//! into [`REFERENCE_PATH`], so the page is generated from the same definitions
//! the binary writes with (`@/inv/single-source-prose`).
//!
//! Every claim on the page is derived rather than restated:
//!
//! - The identifier widths come from the constants that enforce them:
//!   [`SHORT_ID_LENGTH`], the width [`Issue::short_id`](crate::domain::Issue::short_id)
//!   truncates to, and [`MIN_ID_PREFIX_LENGTH`], the minimum
//!   [`resolve_issue_id`](crate::storage::IssueStore::resolve_issue_id) accepts.
//! - The event-log sample lines are [`EventTag::sample`] records encoded with
//!   `serde_json::to_string` — the encoding
//!   [`append_event`](crate::storage::IssueStore::append_event) writes to
//!   `events.jsonl`. The tag vocabulary is NOT restated here: it is the event
//!   catalog's own projection ([`crate::domain::event_catalog`]), which this page
//!   links to.
//! - The gate-run path comes from [`JsonFileStorage::result_path`], the single
//!   source of a run's on-disk location; the record's example is a
//!   [`GateRunResult`] encoded with `serde_json::to_string_pretty`, the encoding
//!   [`save_gate_run_result`](crate::storage::IssueStore::save_gate_run_result)
//!   writes; and each field's presence when unset is read off those encodings.
//! - [`GateRunField`] names the record's fields. Conformance tests hold it against
//!   the property list schemars derives from [`GateRunResult`] itself and against
//!   the serialized record, so a field added to the record fails the suite until
//!   the page documents it, and a golden test asserts the committed page equals
//!   the projection.

use crate::domain::{
    EventTag, GateFinding, GateFindings, GateRunResult, GateRunStatus, GateStage,
    GATE_RUN_SCHEMA_VERSION, SHORT_ID_LENGTH,
};
use crate::storage::{JsonFileStorage, MIN_ID_PREFIX_LENGTH};
use anyhow::Result;
use chrono::{DateTime, Utc};

/// Repo-relative path of the committed reference this module projects.
pub const REFERENCE_PATH: &str = "docs/reference/storage-records.md";

/// The page states that a short id is always long enough to hand back as an id
/// prefix. That holds only while the short id is at least as wide as the
/// resolvable minimum, so the claim is checked at compile time against the two
/// constants it is drawn from.
const _: () = assert!(SHORT_ID_LENGTH >= MIN_ID_PREFIX_LENGTH);

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
    /// `tree_dirty`
    TreeDirty,
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
}

impl GateRunField {
    /// Every field of the record, in [`GateRunResult`] declaration order.
    pub const ALL: [GateRunField; 19] = [
        GateRunField::SchemaVersion,
        GateRunField::RunId,
        GateRunField::GateKey,
        GateRunField::Stage,
        GateRunField::IssueId,
        GateRunField::Commit,
        GateRunField::Branch,
        GateRunField::TreeDirty,
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
            GateRunField::TreeDirty => "tree_dirty",
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
                "The run's identifier, and the name of its directory under `gate-runs/`. Each \
                 execution mints a fresh UUID v4, so a rerun records a new directory instead of \
                 overwriting the previous run."
                    .to_string()
            }
            GateRunField::GateKey => {
                "Key of the gate that ran, as registered in `.jit/gates.toml`.".to_string()
            }
            GateRunField::Stage => "Stage the gate ran at: `precheck` or `postcheck`.".to_string(),
            GateRunField::IssueId => {
                "Full id of the issue the run is about. An issue's runs are selected by matching \
                 this field across the run directories, so `gate-runs/` is flat rather than \
                 nested per issue."
                    .to_string()
            }
            GateRunField::Commit => {
                "Git commit the run was taken at, when the working directory is a git \
                 repository."
                    .to_string()
            }
            GateRunField::Branch => {
                "Git branch the run was taken on, when the working directory is a git \
                 repository."
                    .to_string()
            }
            GateRunField::TreeDirty => {
                "Whether the working tree differed from `commit` when the checker started: \
                 `true` if it carried uncommitted or untracked changes, `false` if it matched \
                 the commit exactly. A `true` run evidences that modified tree rather than the \
                 commit alone. `null` when no cleanliness value provably describes the recorded \
                 commit: there was no commit to compare against (not a git repository, or no \
                 commits yet), or `HEAD` moved through every paired probe attempt while the \
                 evidence was being taken, so the tree state is recorded as unknown rather than \
                 paired with a commit it might not describe."
                    .to_string()
            }
            GateRunField::Status => {
                "The run's verdict: `passed`, `failed`, `error`, `pending`, or `skipped`. For an \
                 executed checker the exit code decides it — `0` passes; a shell that could not \
                 run the command (`126`, `127`) and a checker killed by a signal or by its \
                 timeout are an `error`; any other code fails."
                    .to_string()
            }
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
        }
    }
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
const SAMPLE_RUN_ID: &str = "7b2b0a4c-2f5a-4d3e-9b1a-6c8f0d5e4a21";

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
        run_id: SAMPLE_RUN_ID.to_string(),
        gate_key: "tests".to_string(),
        stage: GateStage::Postcheck,
        issue_id: "9d1f6c02-4a77-4f2b-8f3d-5e0b7a1c8e64".to_string(),
        commit: Some("9f1c0f0a2b3c4d5e6f708192a3b4c5d6e7f80910".to_string()),
        branch: Some("main".to_string()),
        tree_dirty: Some(false),
        status: GateRunStatus::Passed,
        started_at: sample_timestamp(),
        completed_at: Some(sample_timestamp()),
        duration_ms: Some(12_480),
        exit_code: Some(0),
        stdout: "test result: ok. 812 passed; 0 failed".to_string(),
        stderr: String::new(),
        command: "cargo test --workspace".to_string(),
        by: Some("agent:worker-1".to_string()),
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
        tree_dirty: None,
        completed_at: None,
        duration_ms: None,
        exit_code: None,
        by: None,
        message: None,
        findings: None,
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
/// The lines are produced the way the log is written — `serde_json::to_string` of
/// an [`Event`](crate::domain::Event), one per line — so the block is an encoding
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
    .map(|tag| Ok(format!("{}\n", serde_json::to_string(&tag.sample())?)))
    .collect()
}

/// Escape the markdown table cell separator so a `|` in a description cannot
/// break the rendered row.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// Render the storage-record reference ([`REFERENCE_PATH`]).
///
/// The returned string is the page's full contents. The identifier widths come
/// from the constants that enforce them, the event-log block from
/// [`EventTag::sample`] records encoded the way the log is written, and the
/// gate-run path, example record, and field table from
/// [`JsonFileStorage::result_path`] and [`GateRunResult`] — so the page cannot
/// drift from the code that writes the files. The conformance test in this module
/// asserts the committed file equals this output.
///
/// # Errors
///
/// Propagates a `serde_json` failure while encoding one of the sample records.
pub fn render_reference_markdown() -> Result<String> {
    let example_run = serde_json::to_string_pretty(&sample_gate_run())?;
    let events = event_log_sample()?;

    // The run's path, straight from the function storage resolves it with, so the
    // page names the layout the code writes rather than a copy of it.
    let result_path = JsonFileStorage::new(".jit").result_path("<run-id>");
    let result_path = result_path.display();

    // One line per rule, so the interpolated minimum cannot skew the wrapping.
    let prefix_rules = [
        "- an input of full-id length (32 hex digits once normalized) is looked up directly, \
         as given: it resolves only in the canonical hyphenated lowercase form the record is \
         stored under, and is not searched for as a prefix;"
            .to_string(),
        format!(
            "- a shorter input must be at least {MIN_ID_PREFIX_LENGTH} characters after \
             normalization; below the minimum it is refused as an argument error (exit code 2, \
             see [Exit Codes](exit-codes.md)) rather than searched for;"
        ),
        "- the prefix must match exactly one id in the repository index. Several matches are \
         refused as ambiguous, and the error lists the candidates."
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

    Ok(format!(
        "<!-- Generated from `crate::storage::reference` — do not edit by hand. -->\n\
         \n\
         # Storage Record Layout\n\
         \n\
         > **Diátaxis Type:** Reference\n\
         \n\
         The shape of an issue identifier, of a line in the event log, and of a recorded\n\
         gate run. This page is generated from the definitions the binary writes with —\n\
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
         lowercases the input and drops its hyphens to measure it, then:\n\
         \n\
         {prefix_rules}\n\
         \n\
         A short id is {SHORT_ID_LENGTH} characters and the minimum is {MIN_ID_PREFIX_LENGTH},\n\
         so a short id printed by one command is always long enough to hand back to the next.\n\
         \n\
         ## Event Log Records\n\
         \n\
         `.jit/events.jsonl` is the event log, in JSON Lines: one JSON object per line,\n\
         each terminated by a newline. Appending a record serializes it to a single line\n\
         and writes it at the end of the file under an exclusive lock; records are never\n\
         rewritten in place, and reading parses the file line by line, skipping blank\n\
         lines. Every record carries a `type` tag, its own `id`, and a `timestamp`; the\n\
         remaining fields are flat on the object and vary by tag.\n\
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
         writes a new directory beside the old one, and the history of a gate on an issue\n\
         is the set of run directories whose record names that issue. The record is\n\
         written as pretty-printed JSON, through the temp-file-and-rename pattern every\n\
         other `.jit/` write uses.\n\
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
         {fields}\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Issue;
    use crate::storage::{InMemoryStorage, InvalidIdPrefixError, IssueStore};
    use schemars::schema_for;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use uuid::Uuid;

    /// Absolute path of the committed reference, resolved from the crate root so
    /// the test is independent of the process working directory.
    fn reference_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(REFERENCE_PATH)
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
    /// writes — the same `serde_json` encoding `save_gate_run_result` persists.
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
        assert_eq!(of(GateRunField::TreeDirty), Some(Presence::NullWhenUnset));
        assert_eq!(of(GateRunField::Findings), Some(Presence::OmittedWhenUnset));
        assert_eq!(of(GateRunField::RunId), Some(Presence::Always));

        let cleared =
            serde_json::to_value(sample_gate_run_minimal()).expect("the cleared run serializes");
        let cleared = cleared.as_object().expect("a record is a JSON object");
        assert!(!cleared.contains_key(GateRunField::Findings.as_str()));
        assert!(cleared[GateRunField::Commit.as_str()].is_null());
    }

    /// REQ-01: an issue's stored id is a UUID v4, and the short id is the first
    /// `SHORT_ID_LENGTH` characters of it — derived on read, never a stored field.
    #[test]
    fn test_issue_id_is_uuid_v4_and_short_id_is_its_prefix() {
        let issue = Issue::new("Probe".to_string(), String::new());

        let parsed = Uuid::parse_str(&issue.id).expect("an issue id is a UUID");
        assert_eq!(parsed.get_version(), Some(uuid::Version::Random));
        assert_eq!(parsed.hyphenated().to_string(), issue.id);

        assert_eq!(issue.short_id().len(), SHORT_ID_LENGTH);
        assert!(issue.id.starts_with(&issue.short_id()));

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

    /// REQ-01: the prefix minimum the page states is the one resolution enforces
    /// — an input below it is refused as an [`InvalidIdPrefixError`], whatever the
    /// index holds.
    #[test]
    fn test_resolution_refuses_a_prefix_below_the_projected_minimum() {
        let storage = InMemoryStorage::new();
        storage.init().expect("in-memory storage initializes");
        let issue = Issue::new("Probe".to_string(), String::new());
        let id = issue.id.clone();
        storage.save_issue(issue).expect("the issue saves");

        let too_short: String = id
            .replace('-', "")
            .chars()
            .take(MIN_ID_PREFIX_LENGTH - 1)
            .collect();
        let error = storage
            .resolve_issue_id(&too_short)
            .expect_err("a prefix below the minimum is refused");
        assert!(error.downcast_ref::<InvalidIdPrefixError>().is_some());

        // At the minimum, resolution searches the index instead of refusing.
        let shortest: String = id
            .replace('-', "")
            .chars()
            .take(MIN_ID_PREFIX_LENGTH)
            .collect();
        assert_eq!(
            storage
                .resolve_issue_id(&shortest)
                .expect("a prefix at the minimum resolves"),
            id
        );
    }

    /// REQ-01: an input of full-id length is looked up **as given**, so it
    /// resolves only in the canonical hyphenated lowercase form — the page must
    /// not promise that a hyphenless or uppercased full id resolves.
    ///
    /// Resolution normalizes the input to *measure* it, then takes the full-id
    /// fast path with the original string; a 32-hex-digit input therefore never
    /// falls through to the prefix search that would have matched it.
    #[test]
    fn test_full_id_resolves_only_in_canonical_form() {
        let storage = InMemoryStorage::new();
        storage.init().expect("in-memory storage initializes");
        let issue = Issue::new("Probe".to_string(), String::new());
        let id = issue.id.clone();
        storage.save_issue(issue).expect("the issue saves");

        assert_eq!(
            storage
                .resolve_issue_id(&id)
                .expect("the canonical full id resolves"),
            id
        );

        assert!(
            storage.resolve_issue_id(&id.replace('-', "")).is_err(),
            "a hyphenless full id is looked up as given, so it does not resolve"
        );
        assert!(
            storage.resolve_issue_id(&id.to_uppercase()).is_err(),
            "an uppercased full id is looked up as given, so it does not resolve"
        );
    }

    /// REQ-03: the page names the path storage writes a run to, taken from
    /// `JsonFileStorage::result_path` rather than reconstructed.
    #[test]
    fn test_render_projects_the_storage_result_path() {
        let page = render_reference_markdown().expect("the reference renders");
        let path = JsonFileStorage::new(".jit").result_path("<run-id>");
        assert!(page.contains(&format!("`{}`", path.display())));
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
            "{REFERENCE_PATH} is stale — regenerate it from `crate::storage::reference` \
             (run: cargo test -p jit storage::reference -- --ignored regenerate)"
        );
    }

    /// Regenerate the committed reference. Ignored by default; run explicitly
    /// after changing the record layout:
    ///   cargo test -p jit storage::reference -- --ignored regenerate
    ///
    /// Writes via the temp-file + atomic-rename pattern (`@/inv/atomic-writes`).
    #[test]
    #[ignore = "writes the committed reference; run explicitly to regenerate"]
    fn test_regenerate_reference_writes_committed_doc() {
        let path = reference_path();
        let tmp = path.with_extension("md.tmp");
        std::fs::write(
            &tmp,
            render_reference_markdown().expect("the reference renders"),
        )
        .expect("should write the storage-records temp file");
        std::fs::rename(&tmp, &path)
            .expect("should atomically replace the storage-records reference");
    }
}
