//! Command-harness tests for `ItemOps::project_render` (REQ-08).
//!
//! Drives the generic projection command directly over `InMemoryStorage` (no
//! subprocess) — the `CommandExecutor` layer the project's testing strategy
//! prefers. Covers the three REQ-08 surfaces: a markdown-first projection
//! round-trip, byte-exactness outside the region markers, and output PARITY for
//! the two migrated projections (`invariants` id-anchor, `rules-and-gates` full),
//! each bound to the live renderer code rather than a committed mirror.

use jit::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
use jit::declarations::invariants::InvariantRegistry;
use jit::declarations::rules::RuleSet;
use jit::declarations::GateRegistry;
use jit::declarations::{GateDefinition, GateMode, GateStage};
use jit::repository_state::render_invariants_markdown;
use jit::repository_state::render_rules_and_gates_markdown;
use jit::repository_state::VirtualPath;
use jit::storage::{InMemoryStorage, IssueStore};
use jit::test_utils::shipped_item_kind_tables;
use std::collections::HashMap;

const INVARIANTS_TOML: &str = r#"
[[invariants]]
id = "sample-invariant"
statement = "Every dependency edge stays acyclic."
kind = "enforced"
enforced-by = "dag-no-cycles"

[[invariants]]
id = "second-invariant"
statement = "Issues prefer functional style."
kind = "advisory"
"#;

const RULES_TOML: &str = r#"
[[rules]]
name = "require-type-label"
description = "Every issue has a type label."
severity = "error"
enforce = true
assert = { require-label = { label = "type:*" } }
"#;

fn manual_gate(key: &str, title: &str, description: &str) -> GateDefinition {
    GateDefinition {
        version: 1,
        key: key.parse().unwrap(),
        title: title.to_string(),
        description: description.to_string(),
        stage: GateStage::Postcheck,
        mode: GateMode::Manual,
        checker: None,
        priority: 100,
        reserved: HashMap::new(),
        inputs: None,
    }
}

/// The body between (exclusive of) the two markers in `content`.
fn region_body(content: &str, begin: &str, end: &str) -> String {
    let after = content.find(begin).expect("begin marker") + begin.len();
    let rest = &content[after..];
    let end_at = rest.find(end).expect("end marker");
    rest[..end_at].trim_matches('\n').to_string()
}

/// REQ-08 surface 1 + 2: a markdown-first kind projects its `- **{id}** — {text}`
/// rows into a region, and everything OUTSIDE the markers is byte-preserved.
#[test]
fn test_markdown_first_projection_round_trip_and_byte_exactness() {
    // `charter` is a custom project-scope markdown-first kind; the default package ships none.
    let config = r#"
[item_kinds.charter]
section = "decision_log"
id-pattern = "[A-Z]-[0-9]+"
markers = []
link-namespaces = ["per"]
scope = "project"
source = "charter.md"
source-of-truth = "markdown-first"

[projection.charter]
kind = "charter"
mode = "region"
target = "AGENTS.md"
style = "id-anchor"
"#;
    let storage = InMemoryStorage::new();
    // The markdown source: a decision-log section whose bullets repeat their id.
    storage.add_worktree_file(
        "charter.md",
        "## Decision Log\n\n- D-1: JSON-in-git storage\n- D-2: Config-declared gates\n",
    );
    // The target: hand-authored content wrapping the jit-managed region.
    let prefix = "# AGENTS.md\n\n### Charter Decisions\n\n";
    let suffix = "\n\n## Commit Conventions\n\nRun cargo fmt.\n";
    let begin = "<!-- jit:charter:begin -->";
    let end = "<!-- jit:charter:end -->";
    let original = format!("{prefix}{begin}\nstale\n{end}{suffix}");
    storage.add_worktree_file("AGENTS.md", &original);

    let executor = memory_executor_declaring(
        storage.clone(),
        MemoryDeclarations {
            config: Some(config),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    let result = executor.items().project_render(Some("charter")).unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].target, "AGENTS.md");
    assert_eq!(result.items[0].count, 2);

    let updated = storage.read_repo_file("AGENTS.md").unwrap().unwrap();
    // The self-id prefix (`D-1: `) is stripped, not doubled (REQ-06 shape).
    assert_eq!(
        region_body(&updated, begin, end),
        "- **D-1** — JSON-in-git storage\n- **D-2** — Config-declared gates"
    );
    // Everything OUTSIDE the markers is byte-identical.
    assert!(
        updated.starts_with(&format!("{prefix}{begin}")),
        "{updated}"
    );
    assert!(updated.ends_with(&format!("{end}{suffix}")), "{updated}");

    let _ = std::fs::remove_dir_all(storage.root());
}

/// Project rendering selects and reports from the captured config, even when
/// the host filesystem holds a different `config.toml`.
#[test]
fn test_project_render_uses_captured_config_when_disk_config_is_stale() {
    // `charter` is a custom project-scope markdown-first kind; the default package ships none.
    let captured_config = r#"
[item_kinds.charter]
section = "decision_log"
id-pattern = "[A-Z]-[0-9]+"
markers = []
link-namespaces = ["per"]
scope = "project"
source = "charter.md"
source-of-truth = "markdown-first"

[projection.charter]
kind = "charter"
mode = "separate-file"
target = "CAPTURED.md"
style = "id-anchor"
"#;
    let stale_config = r#"
[projection.stale]
kind = "stale"
mode = "separate-file"
target = "STALE.md"
style = "id-anchor"
"#;
    let storage = InMemoryStorage::new();
    storage.add_worktree_file(
        "charter.md",
        "## Decision Log\n\n- D-1: Captured authority\n",
    );

    let executor = memory_executor_declaring(
        storage.clone(),
        MemoryDeclarations {
            config: Some(captured_config),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    std::fs::write(storage.root().join("config.toml"), stale_config).unwrap();
    let result = executor.items().project_render(Some("charter")).unwrap();

    assert_eq!(result.items[0].target, "CAPTURED.md");
    assert_eq!(result.items[0].count, 1);
    assert_eq!(
        storage.read_repo_file("CAPTURED.md").unwrap().unwrap(),
        "- **D-1** — Captured authority\n"
    );
    assert!(storage.read_repo_file("STALE.md").unwrap().is_none());
    let _ = std::fs::remove_dir_all(storage.root());
}

/// REQ-08 surface 3a: the migrated `invariants` (id-anchor) projection reproduces
/// the typed invariant id-anchor render, computed LIVE from the same registry.
#[test]
fn test_invariants_projection_parity_with_typed_render() {
    let config = format!(
        "{kinds}\n[projection.invariants]\nkind = \"invariant\"\n\
         mode = \"separate-file\"\ntarget = \".jit/invariants.md\"\nstyle = \"id-anchor\"\n",
        kinds = shipped_item_kind_tables(&["invariant"])
    );
    let storage = InMemoryStorage::new();
    let executor = memory_executor_declaring(
        storage.clone(),
        MemoryDeclarations {
            config: Some(&config),
            invariants: Some(INVARIANTS_TOML),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );

    executor.items().project_render(Some("invariants")).unwrap();
    let written = storage
        .read_repo_file(".jit/invariants.md")
        .unwrap()
        .expect("separate-file target written");

    // Parity oracle: an independent code path (the typed id-anchor render over the
    // same registry), not a committed mirror.
    let registry = InvariantRegistry::from_toml_str(INVARIANTS_TOML).unwrap();
    let expected = render_invariants_markdown(&registry, jit::config::ProjectionStyle::IdAnchor);
    assert_eq!(written, expected);

    let _ = std::fs::remove_dir_all(storage.root());
}

/// REQ-08 surface 3b: the migrated `rules-and-gates` (full) projection reproduces
/// the typed rule+gate full render, computed LIVE from the effective registries.
#[test]
fn test_rules_and_gates_projection_parity_with_typed_render() {
    let config = format!(
        "{kinds}\n[projection.rules-and-gates]\nkind = [\"rule\", \"gate\"]\n\
         mode = \"separate-file\"\ntarget = \".jit/rules-and-gates.md\"\nstyle = \"full\"\n",
        kinds = shipped_item_kind_tables(&["rule", "gate"])
    );
    let gates_toml = "[[gates]]\nkey = \"cargo-ci\"\ntitle = \"Cargo CI\"\ndescription = \"fmt + clippy\"\nstage = \"postcheck\"\nmode = \"manual\"\n";
    let storage = InMemoryStorage::new();
    let mut registry = GateRegistry::default();
    registry.gates.insert(
        "cargo-ci".parse().unwrap(),
        manual_gate("cargo-ci", "Cargo CI", "fmt + clippy"),
    );
    let executor = memory_executor_declaring(
        storage.clone(),
        MemoryDeclarations {
            config: Some(&config),
            rules: Some(RULES_TOML),
            gates: Some(gates_toml),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    executor
        .items()
        .project_render(Some("rules-and-gates"))
        .unwrap();
    let written = storage
        .read_repo_file(".jit/rules-and-gates.md")
        .unwrap()
        .expect("separate-file target written");

    // Parity oracle: the typed render over the same effective registries.
    let namespaces = jit::config_manager::namespaces_from_config(&toml::from_str(&config).unwrap());
    let effective = jit::repository_state::reconcile_default_rules_with_config(
        RuleSet::parse(RULES_TOML, None, std::iter::empty()).unwrap(),
        &namespaces,
    )
    .unwrap();
    let expected =
        render_rules_and_gates_markdown(&effective, &registry, jit::config::ProjectionStyle::Full);
    assert_eq!(written, expected);
    assert!(written.contains("@/rule/require-type-label"));
    assert!(written.contains("@/gate/cargo-ci"));

    let _ = std::fs::remove_dir_all(storage.root());
}

/// REQ-07: an unknown kind under a projection is a typed error, and nothing is
/// written.
#[test]
fn test_unknown_kind_is_error() {
    let config = "\
[projection.bad]
kind = \"nonexistent\"
mode = \"separate-file\"
target = \".jit/out.md\"
style = \"id-anchor\"
";
    let storage = InMemoryStorage::new();
    let executor = memory_executor_declaring(
        storage.clone(),
        MemoryDeclarations {
            config: Some(config),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    let err = executor.items().project_render(Some("bad")).unwrap_err();
    assert!(
        err.to_string().contains("bad") || err.chain().any(|c| c.to_string().contains("unknown")),
        "unexpected error: {err:#}"
    );
    assert!(storage.read_repo_file(".jit/out.md").unwrap().is_none());
    let _ = std::fs::remove_dir_all(storage.root());
}

/// REQ-07 (F1): a projection that declares no `target` is a typed
/// `ProjectionError::MissingTarget` naming the projection, and no default
/// `.jit/<name>.md` file is silently written.
#[test]
fn test_missing_target_is_typed_error_naming_projection() {
    let config = format!(
        "{kinds}\n[projection.invariants]\nkind = \"invariant\"\nstyle = \"id-anchor\"\n",
        kinds = shipped_item_kind_tables(&["invariant"])
    );
    let storage = InMemoryStorage::new();
    let executor = memory_executor_declaring(
        storage.clone(),
        MemoryDeclarations {
            config: Some(&config),
            invariants: Some(INVARIANTS_TOML),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );

    let err = executor
        .items()
        .project_render(Some("invariants"))
        .unwrap_err();
    let typed = err.downcast_ref::<jit::repository_state::ProjectionError>();
    assert!(
        matches!(
            typed,
            Some(jit::repository_state::ProjectionError::MissingTarget { projection })
                if projection == "invariants"
        ),
        "expected MissingTarget naming the projection, got {err:#}"
    );
    // The removed silent default `.jit/invariants.md` is NOT created.
    assert!(storage
        .read_repo_file(".jit/invariants.md")
        .unwrap()
        .is_none());
    let _ = std::fs::remove_dir_all(storage.root());
}

/// REQ-07 (F2): rendering ALL projections is two-phase — with a valid first
/// projection and a second whose region markers are absent, the render writes
/// NOTHING. The valid first target stays byte-identical because every projection
/// materializes before any target is written.
#[test]
fn test_two_phase_render_writes_nothing_when_a_later_projection_fails() {
    let config = format!(
        "{kinds}\n\
         [projection.a-good]\nkind = \"invariant\"\nmode = \"region\"\n\
         target = \"FIRST.md\"\nstyle = \"id-anchor\"\n\
         [projection.b-bad]\nkind = \"invariant\"\nmode = \"region\"\n\
         target = \"SECOND.md\"\nstyle = \"id-anchor\"\n",
        kinds = shipped_item_kind_tables(&["invariant"])
    );
    let storage = InMemoryStorage::new();
    // FIRST.md carries `a-good`'s default region markers (rendering succeeds);
    // SECOND.md carries NONE, so `b-bad` fails on its absent begin marker.
    let first_original =
        "# First\n\n<!-- jit:a-good:begin -->\nstale\n<!-- jit:a-good:end -->\n\n## Tail\n";
    let second_original = "# Second\n\nNo managed region here.\n";
    storage.add_worktree_file("FIRST.md", first_original);
    storage.add_worktree_file("SECOND.md", second_original);

    let executor = memory_executor_declaring(
        storage.clone(),
        MemoryDeclarations {
            config: Some(&config),
            invariants: Some(INVARIANTS_TOML),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    // `a-good` sorts before `b-bad`, so it materializes (phase 1) before `b-bad`
    // fails — yet phase 2 never runs, so nothing is written anywhere.
    let err = executor.items().project_render(None).unwrap_err();
    // `b-bad`'s target lacks its managed region, so the managed-document engine
    // rejects the composition with a typed required-region-absent fault before the
    // delta is applied (it keeps the validation exit code, like the old marker
    // error it replaced).
    let typed = err.downcast_ref::<jit::repository_state::ManagedDocumentError>();
    assert!(
        matches!(
            typed,
            Some(jit::repository_state::ManagedDocumentError::RequiredRegionAbsent(region))
                if region == "b-bad"
        ),
        "expected RequiredRegionAbsent(\"b-bad\"), got {err:#}"
    );
    assert_eq!(
        storage.read_repo_file("FIRST.md").unwrap().unwrap(),
        first_original,
        "the valid first projection's target must be byte-identical"
    );
    assert_eq!(
        storage.read_repo_file("SECOND.md").unwrap().unwrap(),
        second_original
    );
    let _ = std::fs::remove_dir_all(storage.root());
}

/// Round-2 F1 (`@/inv/domain-agnostic`): `full`-style dispatch keys on the kind's
/// DECLARED registry source, not its name. A RENAMED registry-first kind whose
/// `source` points at the invariants store full-renders identically to what the
/// built-in `invariant`-named projection would produce — no engine change needed
/// to rename the kind.
#[test]
fn test_full_style_dispatches_on_registry_source_not_kind_name() {
    // `house-rules` is a custom kind name over the shipped invariant declaration:
    // a different NAME, the SAME registry source (`.jit/invariants.toml`).
    let renamed_kind = shipped_item_kind_tables(&["invariant"])
        .replace("item_kinds.invariant", "item_kinds.house-rules");
    let config = format!(
        "{renamed_kind}\n[projection.house]\nkind = \"house-rules\"\n\
         mode = \"separate-file\"\ntarget = \".jit/house.md\"\nstyle = \"full\"\n"
    );
    // Full style renders `config.invariants`, loaded from the `.jit/invariants.toml`
    // store on the filesystem root — seed it there. The pre-write source-existence
    // guard probes the same store over `read_repo_file`, so seed the repo-file map
    // too.
    let storage = InMemoryStorage::new();
    let executor = memory_executor_declaring(
        storage.clone(),
        MemoryDeclarations {
            config: Some(&config),
            invariants: Some(INVARIANTS_TOML),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );

    executor.items().project_render(Some("house")).unwrap();
    let written = storage
        .read_repo_file(".jit/house.md")
        .unwrap()
        .expect("separate-file target written");

    // Byte-identical to the built-in `full` invariant render over the same registry
    // — the exact output the built-in `invariant`-named projection would write.
    let registry = InvariantRegistry::from_toml_str(INVARIANTS_TOML).unwrap();
    let expected = render_invariants_markdown(&registry, jit::config::ProjectionStyle::Full);
    assert_eq!(written, expected);
    assert!(written.contains("## Project invariants"));

    let _ = std::fs::remove_dir_all(storage.root());
}

/// F2 (`@/charter/D-6`): full style shares the id-anchor project-scope guard. A
/// `full` projection over an ISSUE-scoped kind is a typed
/// `ProjectionError::NotProjectScoped`, and nothing is written.
#[test]
fn test_full_style_issue_scoped_kind_is_typed_error_and_writes_nothing() {
    // `decision` is an issue-scoped markdown-first kind: it has no project-scope
    // registry, so it cannot back a full-style projection.
    let issue_scoped_kind = shipped_item_kind_tables(&["decision"]);
    let config = format!(
        "{issue_scoped_kind}\n[projection.decisions]\nkind = \"decision\"\n\
         mode = \"separate-file\"\ntarget = \".jit/decisions.md\"\nstyle = \"full\"\n"
    );
    let storage = InMemoryStorage::new();
    let executor = memory_executor_declaring(
        storage.clone(),
        MemoryDeclarations {
            config: Some(&config),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );

    let err = executor
        .items()
        .project_render(Some("decisions"))
        .unwrap_err();
    let typed = err.downcast_ref::<jit::repository_state::ProjectionError>();
    assert!(
        matches!(
            typed,
            Some(jit::repository_state::ProjectionError::NotProjectScoped { kind })
                if kind == "decision"
        ),
        "expected NotProjectScoped naming the kind, got {err:#}"
    );
    assert!(storage
        .read_repo_file(".jit/decisions.md")
        .unwrap()
        .is_none());
    let _ = std::fs::remove_dir_all(storage.root());
}

/// F2 (REQ-07): the full style shares the anchor style's missing-source guard.
/// A full-style projection whose declared registry store is absent over the
/// `read_repo_file` boundary is a typed `ProjectionError::SourceNotFound` raised
/// BEFORE any write, never a silent empty render from the defaulted in-memory
/// registry.
#[test]
fn test_full_style_missing_registry_source_errors_pre_write() {
    let config = format!(
        "{kinds}\n[projection.invariants]\nkind = \"invariant\"\n\
         mode = \"separate-file\"\ntarget = \".jit/invariants.md\"\nstyle = \"full\"\n",
        kinds = shipped_item_kind_tables(&["invariant"])
    );
    // The `.jit/invariants.toml` store is absent: config.invariants loads
    // empty (a missing store is indistinguishable from an empty one at render
    // time), and the pre-write existence probe over `read_repo_file` finds nothing.
    let storage = InMemoryStorage::new();
    let executor = memory_executor_declaring(
        storage.clone(),
        MemoryDeclarations {
            config: Some(&config),
            after_initialization: true,
            absent: &[VirtualPath::INVARIANTS],
            ..MemoryDeclarations::default()
        },
    );

    let err = executor
        .items()
        .project_render(Some("invariants"))
        .unwrap_err();
    let typed = err.downcast_ref::<jit::repository_state::ProjectionError>();
    assert!(
        matches!(
            typed,
            Some(jit::repository_state::ProjectionError::SourceNotFound { path, kind })
                if path == ".jit/invariants.toml" && kind == "invariant"
        ),
        "expected SourceNotFound for the absent store, got {err:#}"
    );
    // No empty block is written to the target.
    assert!(storage
        .read_repo_file(".jit/invariants.md")
        .unwrap()
        .is_none());
    let _ = std::fs::remove_dir_all(storage.root());
}
