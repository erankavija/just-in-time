//! Project the rule and gate registries into a CONFIGURABLE documentation target.
//!
//! The rules/gates analogue of the invariant projection
//! ([`projection`](crate::validation::projection)): it renders BOTH the effective
//! [`RuleSet`] and the [`GateRegistry`] into one reference document and writes it
//! into a config-selected target, reusing the SAME projection primitives:
//!
//! - the [`ProjectionMode`] / [`ProjectionStyle`] configuration knobs,
//! - the region-splice function [`splice_region`], and
//! - the shared, typed [`ProjectionError`] plus the storage atomic-write boundary.
//!
//! This is a NEW, generically-typed renderer — [`render_rules_and_gates_markdown`]
//! takes the two registries directly, rather than reusing the invariant-typed
//! [`render_invariants_markdown`](crate::validation::projection::render_invariants_markdown),
//! which is specialized to the invariant registry. The orchestrator
//! [`project_rules_and_gates`] mirrors
//! [`project_invariants`](crate::validation::projection::project_invariants) as a
//! pattern (config-driven target, mode branch, atomic write) without calling it.
//!
//! Every rule/gate is addressed by its canonical kind-segmented form —
//! `@/rule/<name>` and `@/gate/<key>` — matching the `[item_kinds.rule]` /
//! `[item_kinds.gate]` registry projection.
//!
//! Rendering is PURE and unit-testable; the orchestrator is the only function that
//! performs I/O.

use crate::config::{ProjectionMode, ProjectionStyle, RulesGatesProjectionConfig};
use crate::storage::{GateRegistry, IssueStore};
use crate::validation::projection::{splice_region, ProjectionError};
use crate::validation::rules::RuleSet;

/// The canonical kind-segmented address of a rule (`@/rule/<name>`).
fn rule_address(name: &str) -> String {
    format!("@/rule/{name}")
}

/// The canonical kind-segmented address of a gate (`@/gate/<key>`).
fn gate_address(key: &str) -> String {
    format!("@/gate/{key}")
}

/// Render `rules` and `gates` into a deterministic reference document in `style`.
///
/// Pure: performs no I/O and reads no configuration beyond the `style` argument.
/// Rules are listed in authored order; gates are sorted by key so the output is
/// deterministic despite the registry's hash-map storage. Every entry is addressed
/// by its canonical kind-segmented form (`@/rule/<name>`, `@/gate/<key>`). The two
/// styles ([`ProjectionStyle`]) differ only in framing:
///
/// - [`ProjectionStyle::Full`] (the default) renders a `## Rules` section of
///   `- **@/rule/{name}** — severity: {severity}, enforce: {bool}` bullets followed
///   by a `## Gates` section of `- **@/gate/{key}** — {title}: {description}`
///   bullets (the `: {description}` suffix is omitted when the description is
///   empty).
/// - [`ProjectionStyle::IdAnchor`] renders a HEADING-LESS bullet list — one
///   `- **{address}** — {display-text}` line per rule (display text = the rule
///   name, the rule kind's registry text-field) then per gate (display text = the
///   gate description, or its title when the description is empty) — for embedding
///   beneath a hand-authored heading.
///
/// An empty registry renders an explicit "none declared" line in its section (Full
/// style) or, when BOTH registries are empty, a single "none declared" line
/// (IdAnchor style), so the projected region is never blank.
///
/// # Examples
///
/// ```
/// use jit::config::ProjectionStyle;
/// use jit::storage::GateRegistry;
/// use jit::validation::rules::RuleSet;
/// use jit::validation::rules_gates_projection::render_rules_and_gates_markdown;
/// use std::path::Path;
///
/// let rules = RuleSet::from_toml_str(
///     "[[rules]]\nname = \"label-format\"\nseverity = \"error\"\nenforce = true\n\
///      assert = { require-label = { label = \"type:*\" } }\n",
///     Path::new("/nonexistent"),
/// )
/// .unwrap();
/// let gates = GateRegistry::default();
///
/// // Full style keeps the section headings and the canonical rule address.
/// let full = render_rules_and_gates_markdown(&rules, &gates, ProjectionStyle::Full);
/// assert!(full.contains("## Rules"));
/// assert!(full.contains("## Gates"));
/// assert!(full.contains("- **@/rule/label-format** — severity: error, enforce: true"));
/// assert!(full.contains("_No gates declared._"));
///
/// // Id-anchor style is heading-less.
/// let anchored = render_rules_and_gates_markdown(&rules, &gates, ProjectionStyle::IdAnchor);
/// assert!(!anchored.contains("## Rules"));
/// assert_eq!(anchored, "- **@/rule/label-format** — label-format\n");
/// ```
pub fn render_rules_and_gates_markdown(
    rules: &RuleSet,
    gates: &GateRegistry,
    style: ProjectionStyle,
) -> String {
    match style {
        ProjectionStyle::Full => render_full(rules, gates),
        ProjectionStyle::IdAnchor => render_id_anchor(rules, gates),
    }
}

/// Gate `(key, gate)` pairs in ascending key order (deterministic).
fn gates_sorted(gates: &GateRegistry) -> Vec<(&String, &crate::domain::Gate)> {
    let mut pairs: Vec<_> = gates.gates.iter().collect();
    pairs.sort_by_key(|(key, _)| *key);
    pairs
}

/// The gate's display text: its description, falling back to its title when the
/// description is empty (so a bullet never trails a bare em-dash).
fn gate_display_text(gate: &crate::domain::Gate) -> &str {
    if gate.description.is_empty() {
        &gate.title
    } else {
        &gate.description
    }
}

/// Render the `full` style: `## Rules` and `## Gates` sections with metadata.
fn render_full(rules: &RuleSet, gates: &GateRegistry) -> String {
    let mut out = String::from("## Rules\n\n");
    if rules.rules.is_empty() {
        out.push_str("_No rules declared._\n");
    } else {
        for rule in &rules.rules {
            out.push_str(&format!(
                "- **{address}** — severity: {severity}, enforce: {enforce}\n",
                address = rule_address(&rule.name),
                severity = rule.severity.token(),
                enforce = rule.enforce,
            ));
        }
    }

    out.push_str("\n## Gates\n\n");
    let sorted = gates_sorted(gates);
    if sorted.is_empty() {
        out.push_str("_No gates declared._\n");
    } else {
        for (key, gate) in sorted {
            let description = if gate.description.is_empty() {
                String::new()
            } else {
                format!(": {}", gate.description)
            };
            out.push_str(&format!(
                "- **{address}** — {title}{description}\n",
                address = gate_address(key),
                title = gate.title,
            ));
        }
    }
    out
}

/// Render the `id-anchor` style: a heading-less `- **{address}** — {text}` list.
///
/// Rules (display text = name) then gates (display text = description or title), in
/// deterministic order. When BOTH registries are empty, a single explicit line is
/// emitted so the projected region is never blank.
fn render_id_anchor(rules: &RuleSet, gates: &GateRegistry) -> String {
    let sorted = gates_sorted(gates);
    if rules.rules.is_empty() && sorted.is_empty() {
        return String::from("_No rules or gates declared._\n");
    }
    let mut out = String::new();
    for rule in &rules.rules {
        out.push_str(&format!(
            "- **{address}** — {text}\n",
            address = rule_address(&rule.name),
            text = rule.name,
        ));
    }
    for (key, gate) in sorted {
        out.push_str(&format!(
            "- **{address}** — {text}\n",
            address = gate_address(key),
            text = gate_display_text(gate),
        ));
    }
    out
}

/// Project `rules` and `gates` into the documentation target described by `config`.
///
/// The orchestrator (the only function here that performs I/O) reads the target
/// path, mode, render style, and delimiters ONLY from `config` — this module
/// contains no documentation-filename literal. It mirrors
/// [`project_invariants`](crate::validation::projection::project_invariants) as a
/// pattern (it does not call it): render → for `separate-file`, atomic-write the
/// whole file; for `region`, read the existing target through the storage
/// boundary, splice the rendered block between the configured delimiters via the
/// shared [`splice_region`] (byte-preserving everything outside), then atomic-write
/// the result. Persistence goes through
/// [`write_repo_file`](crate::storage::IssueStore::write_repo_file), which
/// path-validates the config-driven target (rejecting absolute/`..`-escaping paths)
/// and writes atomically. A missing target or missing/malformed delimiters is a
/// typed [`ProjectionError`] — the file is never silently clobbered.
///
/// Returns the repo-relative path that was written.
///
/// # Examples
///
/// ```no_run
/// use jit::config::RulesGatesProjectionConfig;
/// use jit::storage::{GateRegistry, JsonFileStorage};
/// use jit::validation::rules::RuleSet;
/// use jit::validation::rules_gates_projection::project_rules_and_gates;
///
/// let store = JsonFileStorage::new(".jit");
/// let cfg = RulesGatesProjectionConfig::default();
/// let rules = RuleSet::empty();
/// let gates = GateRegistry::default();
/// // Writes the rendered registries to the configured (default jit-owned) target.
/// let written = project_rules_and_gates(&store, &cfg, &rules, &gates).unwrap();
/// println!("projected rules and gates to {written}");
/// ```
pub fn project_rules_and_gates<S: IssueStore>(
    store: &S,
    config: &RulesGatesProjectionConfig,
    rules: &RuleSet,
    gates: &GateRegistry,
) -> Result<String, ProjectionError> {
    let target = config.target();
    let rendered = render_rules_and_gates_markdown(rules, gates, config.style());

    let content = match config.mode() {
        ProjectionMode::SeparateFile => rendered,
        ProjectionMode::Region => {
            let existing = store
                .read_repo_file(target)
                .map_err(|source| ProjectionError::Read {
                    path: target.to_string(),
                    source,
                })?
                .ok_or_else(|| ProjectionError::TargetNotFound {
                    path: target.to_string(),
                })?;
            splice_region(
                &existing,
                &rendered,
                config.region_begin(),
                config.region_end(),
            )?
        }
    };

    store
        .write_repo_file(target, &content)
        .map_err(|source| ProjectionError::Write {
            path: target.to_string(),
            source,
        })?;
    Ok(target.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Gate, GateMode, GateStage};
    use crate::storage::{JsonFileStorage, PathReadError};
    use std::collections::HashMap;
    use std::path::Path;

    fn ruleset() -> RuleSet {
        RuleSet::from_toml_str(
            r#"
[[rules]]
name = "label-format"
severity = "error"
enforce = true
assert = { require-label = { label = "type:*" } }

[[rules]]
name = "orphan-leaf"
severity = "warn"
enforce = false
assert = { require-section = { heading = "Goal" } }
"#,
            Path::new("/nonexistent"),
        )
        .unwrap()
    }

    fn gate(key: &str, title: &str, description: &str) -> Gate {
        Gate {
            version: 1,
            key: key.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Manual,
            checker: None,
            priority: 100,
            reserved: HashMap::new(),
            auto: false,
            example_integration: None,
        }
    }

    fn gate_registry() -> GateRegistry {
        let mut registry = GateRegistry::default();
        // Inserted out of key order to prove the render sorts by key.
        registry.gates.insert(
            "cargo-ci".to_string(),
            gate("cargo-ci", "Cargo CI", "fmt + clippy + tests"),
        );
        registry.gates.insert(
            "breakdown-review".to_string(),
            gate(
                "breakdown-review",
                "AI Breakdown Review",
                "adversarial review",
            ),
        );
        registry
    }

    #[test]
    fn test_render_full_lists_rules_then_gates_deterministically() {
        let md =
            render_rules_and_gates_markdown(&ruleset(), &gate_registry(), ProjectionStyle::Full);
        // Section headings frame the two registries.
        let rules_at = md.find("## Rules").unwrap();
        let gates_at = md.find("## Gates").unwrap();
        assert!(rules_at < gates_at);
        // Rules in authored order with canonical addresses + metadata.
        assert!(md.contains("- **@/rule/label-format** — severity: error, enforce: true"));
        assert!(md.contains("- **@/rule/orphan-leaf** — severity: warn, enforce: false"));
        // Gates sorted by key (breakdown-review before cargo-ci despite insert order).
        let brk = md.find("@/gate/breakdown-review").unwrap();
        let cci = md.find("@/gate/cargo-ci").unwrap();
        assert!(brk < cci, "gates must be key-sorted: {md}");
        assert!(md.contains("- **@/gate/cargo-ci** — Cargo CI: fmt + clippy + tests"));
        // Deterministic: identical input renders identical output.
        assert_eq!(
            md,
            render_rules_and_gates_markdown(&ruleset(), &gate_registry(), ProjectionStyle::Full)
        );
    }

    #[test]
    fn test_render_full_empty_registries_have_explicit_lines() {
        let md = render_rules_and_gates_markdown(
            &RuleSet::empty(),
            &GateRegistry::default(),
            ProjectionStyle::Full,
        );
        assert!(md.contains("## Rules"));
        assert!(md.contains("_No rules declared._"));
        assert!(md.contains("## Gates"));
        assert!(md.contains("_No gates declared._"));
    }

    #[test]
    fn test_render_full_gate_without_description_omits_suffix() {
        let mut registry = GateRegistry::default();
        registry
            .gates
            .insert("bare".to_string(), gate("bare", "Bare Gate", ""));
        let md =
            render_rules_and_gates_markdown(&RuleSet::empty(), &registry, ProjectionStyle::Full);
        // No trailing "`: `" when the description is empty.
        assert!(md.contains("- **@/gate/bare** — Bare Gate\n"), "{md}");
    }

    #[test]
    fn test_render_id_anchor_is_heading_less() {
        let md = render_rules_and_gates_markdown(
            &ruleset(),
            &gate_registry(),
            ProjectionStyle::IdAnchor,
        );
        assert!(!md.contains("## Rules"));
        assert!(!md.contains("## Gates"));
        assert!(!md.contains("severity:"));
        // Rules use the name as display text; gates use the description.
        assert_eq!(
            md,
            "- **@/rule/label-format** — label-format\n\
             - **@/rule/orphan-leaf** — orphan-leaf\n\
             - **@/gate/breakdown-review** — adversarial review\n\
             - **@/gate/cargo-ci** — fmt + clippy + tests\n"
        );
    }

    #[test]
    fn test_render_id_anchor_both_empty_has_single_explicit_line() {
        let md = render_rules_and_gates_markdown(
            &RuleSet::empty(),
            &GateRegistry::default(),
            ProjectionStyle::IdAnchor,
        );
        assert_eq!(md, "_No rules or gates declared._\n");
    }

    #[test]
    fn test_render_id_anchor_gate_without_description_falls_back_to_title() {
        let mut registry = GateRegistry::default();
        registry
            .gates
            .insert("bare".to_string(), gate("bare", "Bare Gate", ""));
        let md = render_rules_and_gates_markdown(
            &RuleSet::empty(),
            &registry,
            ProjectionStyle::IdAnchor,
        );
        assert_eq!(md, "- **@/gate/bare** — Bare Gate\n");
    }

    #[test]
    fn test_project_separate_file_writes_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        let cfg = RulesGatesProjectionConfig {
            mode: Some(ProjectionMode::SeparateFile),
            target: Some("docs/rules-and-gates.md".to_string()),
            ..Default::default()
        };
        let written = project_rules_and_gates(&store, &cfg, &ruleset(), &gate_registry()).unwrap();
        assert_eq!(written, "docs/rules-and-gates.md");

        let on_disk = std::fs::read_to_string(dir.path().join("docs/rules-and-gates.md")).unwrap();
        assert!(on_disk.contains("@/rule/label-format"));
        assert!(on_disk.contains("@/gate/cargo-ci"));
        // No leftover temp file (atomic temp+rename leaves only the target).
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("docs"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no .tmp temp file should remain");
    }

    #[test]
    fn test_project_region_byte_preserves_surrounding_file() {
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        let begin = "<!-- jit:rules-and-gates:begin -->";
        let end = "<!-- jit:rules-and-gates:end -->";
        let prefix = "# Rules and Gates\n\nHand-authored intro.\n\n";
        let suffix = "\n\n## Footer\n\nHand-authored outro.\n";
        let original = format!("{prefix}{begin}\nstale\n{end}{suffix}");
        std::fs::write(dir.path().join("REF.md"), &original).unwrap();

        let cfg = RulesGatesProjectionConfig {
            mode: Some(ProjectionMode::Region),
            target: Some("REF.md".to_string()),
            region_begin: Some(begin.to_string()),
            region_end: Some(end.to_string()),
            ..Default::default()
        };
        project_rules_and_gates(&store, &cfg, &ruleset(), &gate_registry()).unwrap();

        let updated = std::fs::read_to_string(dir.path().join("REF.md")).unwrap();
        // Bytes OUTSIDE the region are byte-identical.
        assert!(updated.starts_with(&format!("{prefix}{begin}")));
        assert!(updated.ends_with(&format!("{end}{suffix}")));
        // Region replaced with the rendered registries.
        assert!(updated.contains("@/rule/label-format"));
        assert!(updated.contains("@/gate/breakdown-review"));
        assert!(!updated.contains("stale"));
    }

    #[test]
    fn test_project_region_missing_target_is_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        let cfg = RulesGatesProjectionConfig {
            mode: Some(ProjectionMode::Region),
            target: Some("MISSING.md".to_string()),
            ..Default::default()
        };
        let err = project_rules_and_gates(&store, &cfg, &ruleset(), &gate_registry()).unwrap_err();
        assert!(matches!(err, ProjectionError::TargetNotFound { .. }));
    }

    #[test]
    fn test_project_separate_file_rejects_escaping_target() {
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        for bad in ["../escape.md", "/tmp/jit-escape.md"] {
            let cfg = RulesGatesProjectionConfig {
                mode: Some(ProjectionMode::SeparateFile),
                target: Some(bad.to_string()),
                ..Default::default()
            };
            let err =
                project_rules_and_gates(&store, &cfg, &ruleset(), &gate_registry()).unwrap_err();
            assert!(
                matches!(
                    err,
                    ProjectionError::Write {
                        source: PathReadError::InvalidPath(_),
                        ..
                    }
                ),
                "escaping target {bad} must be rejected with InvalidPath, got {err:?}"
            );
        }
        assert!(!dir.path().join("../escape.md").exists());
    }

    #[test]
    fn test_default_config_targets_separate_jit_owned_file() {
        let cfg = RulesGatesProjectionConfig::default();
        assert_eq!(cfg.mode(), ProjectionMode::SeparateFile);
        assert_eq!(cfg.target(), ".jit/rules-and-gates.md");
    }
}
