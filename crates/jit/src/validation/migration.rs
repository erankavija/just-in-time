//! `jit init` config -> rules migration and complete default-rules scaffolding
//! (DR §8.2, §8.4; decisions D5/D6/D7).
//!
//! # File becomes the operative single source of truth
//!
//! Unlike the superseded partial attempt (which split rules across `config.toml`
//! and a *subset* `rules.toml`), this migration serializes the COMPLETE default
//! ruleset into `.jit/rules.toml` (+ `.jit/schemas/*.json`) and strips ALL the
//! migrated enforcement keys from `config.toml`. After it runs,
//! [`effective_rules`](crate::commands::CommandExecutor::effective_rules) reads
//! the file as the sole source — there is no parallel in-code enforcement source.
//!
//! The complete ruleset is computed by
//! [`default_ruleset`](crate::validation::defaults::default_ruleset) from the
//! repo's `[validation]` + `[namespaces]` config and rendered by
//! [`serialize_ruleset`](crate::validation::serialize::serialize_ruleset), so the
//! serialized file reproduces today's behavior exactly (accept/reject +
//! warn/block + severity), proven by the §5 parity battery.
//!
//! # Behavior by repo state (D5)
//!
//! - **Fresh / legacy repo, no `rules.toml`:** write the complete serialized
//!   ruleset + schema files, then strip the migrated keys from `config.toml`
//!   (removing an emptied `[validation]` header). A FRESH repo's config template
//!   already ships post-migration, so its strip removes nothing surprising and
//!   the caller suppresses the "migrated N keys" message; a LEGACY repo's strip
//!   reports the removed keys.
//! - **Coexistence** (`rules.toml` already exists AND legacy keys remain): never
//!   clobber the user file — APPEND each default rule whose name is not already
//!   present, warn about skipped names, then strip the legacy keys.
//! - **Already-migrated** (`rules.toml` present, no legacy keys): no-op.
//!
//! Reuses the superseded branch's `toml_edit` comment-preserving key stripping,
//! atomic temp+rename writes, and idempotency guard; the deprecated-key scanner
//! lives in [`crate::config`].

use std::path::Path;

use anyhow::{Context, Result};

use crate::config::{deprecated_keys_in_config, JitConfig, ValidationConfig};
use crate::domain::LabelNamespaces;
use crate::validation::defaults::default_ruleset;
use crate::validation::rules::RuleSet;
use crate::validation::serialize::{serialize_ruleset, SchemaFile, SerializedRuleSet};

/// Which of the four repo states [`migrate_or_scaffold`] acted on (D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationState {
    /// No `rules.toml`, no legacy keys: a complete `rules.toml` was scaffolded
    /// from the in-code intended defaults (no strip, no migration message).
    FreshScaffold,
    /// No `rules.toml`, legacy keys present: serialized the complete ruleset from
    /// the live config and stripped the legacy keys.
    LegacyMigrated,
    /// `rules.toml` present AND legacy keys present: appended missing defaults by
    /// name (no clobber) and stripped the legacy keys.
    Coexistence,
    /// `rules.toml` present AND no legacy keys: nothing to do (true no-op).
    AlreadyMigrated,
}

/// The outcome of running [`migrate_or_scaffold`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationOutcome {
    /// Which repo state was handled.
    pub state: MigrationState,
    /// Fully-qualified legacy keys stripped from `config.toml` (empty for a fresh
    /// scaffold or an already-migrated repo).
    pub stripped_keys: Vec<String>,
    /// Default rule names skipped during coexistence because the user's
    /// `rules.toml` already defined them (empty otherwise).
    pub skipped_existing: Vec<String>,
}

impl MigrationOutcome {
    /// Whether this run performed no changes (already-migrated no-op).
    pub fn is_noop(&self) -> bool {
        self.state == MigrationState::AlreadyMigrated
    }
}

/// Serialize the COMPLETE default ruleset for a repo (D4/D5).
///
/// Pure: the rendered `rules.toml` body + the `.jit/schemas/*.json` files it
/// references. The ruleset is `default_ruleset(validation, namespaces)`.
pub fn serialize_complete_ruleset(
    config: &JitConfig,
    namespaces: &LabelNamespaces,
) -> SerializedRuleSet {
    let validation = config
        .validation
        .clone()
        .unwrap_or_else(empty_validation_config);
    let set = default_ruleset(&validation, namespaces);
    serialize_ruleset(&set)
}

fn empty_validation_config() -> ValidationConfig {
    ValidationConfig {
        strictness: None,
        default_type: None,
        require_type_label: None,
        label_regex: None,
        reject_malformed_labels: None,
        enforce_namespace_registry: None,
        warn_orphaned_leaves: None,
        warn_strategic_consistency: None,
    }
}

/// Run the `jit init` migration / scaffold under `jit_root` (the `.jit` dir),
/// dispatching on the repo's state (D5).
///
/// - `config`/`namespaces` are the repo's LIVE on-disk config (post-template for
///   a fresh repo, so they carry NO constraints; for a legacy repo they carry the
///   live enforcement keys). They drive the complete ruleset for the legacy and
///   coexistence paths and the legacy-key detection.
/// - `fresh_defaults` is the in-code INTENDED default config (carrying the rich
///   constraints, decision D6). It drives the complete ruleset ONLY for a fresh
///   scaffold, where the on-disk config is intentionally clean and therefore
///   cannot reproduce today's checks on its own.
///
/// The four states (keyed on `rules.toml` presence AND legacy-key presence):
///
/// | rules.toml | legacy keys | state            | action                          |
/// |------------|-------------|------------------|---------------------------------|
/// | absent     | none        | FreshScaffold    | write complete file from `fresh_defaults` |
/// | absent     | present     | LegacyMigrated   | write complete file from `config`, strip keys |
/// | present    | present     | Coexistence      | append missing defaults by name, strip keys |
/// | present    | none        | AlreadyMigrated  | NO-OP (no append, no warnings, no strip) |
///
/// The AlreadyMigrated guard is what makes a repeated `jit init` a true no-op
/// (no spurious "skipped" warnings).
pub fn migrate_or_scaffold(
    jit_root: &Path,
    config: &JitConfig,
    namespaces: &LabelNamespaces,
    fresh_defaults: &(JitConfig, LabelNamespaces),
) -> Result<MigrationOutcome> {
    let rules_path = jit_root.join("rules.toml");
    let config_path = jit_root.join("config.toml");
    let file_present = rules_path.exists();
    let legacy_keys = detect_legacy_keys(&config_path);

    let (state, stripped_keys, skipped_existing) = match (file_present, legacy_keys.is_empty()) {
        // Already-migrated: a present file with no stale keys is a true no-op.
        (true, true) => (MigrationState::AlreadyMigrated, Vec::new(), Vec::new()),

        // Coexistence: preserve the user file, append missing defaults by name,
        // then strip the stale keys. The complete ruleset is derived from the
        // LIVE config (it still carries the legacy constraints).
        (true, false) => {
            let serialized = serialize_complete_ruleset(config, namespaces);
            let skipped = append_missing_rules(jit_root, &rules_path, &serialized)?;
            strip_keys_from_config(&config_path, &legacy_keys)?;
            (MigrationState::Coexistence, legacy_keys, skipped)
        }

        // Fresh scaffold: write the complete file from the in-code intended
        // defaults (the on-disk config is clean). No keys to strip.
        (false, true) => {
            let (fresh_config, fresh_ns) = fresh_defaults;
            let serialized = serialize_complete_ruleset(fresh_config, fresh_ns);
            write_complete_ruleset(jit_root, &rules_path, &serialized)?;
            (MigrationState::FreshScaffold, Vec::new(), Vec::new())
        }

        // Legacy migration: write the complete file from the LIVE config (which
        // carries the keys), then strip them.
        (false, false) => {
            let serialized = serialize_complete_ruleset(config, namespaces);
            write_complete_ruleset(jit_root, &rules_path, &serialized)?;
            strip_keys_from_config(&config_path, &legacy_keys)?;
            (MigrationState::LegacyMigrated, legacy_keys, Vec::new())
        }
    };

    Ok(MigrationOutcome {
        state,
        stripped_keys,
        skipped_existing,
    })
}

/// Detect the legacy enforcement keys present in `config.toml` (D7), as
/// fully-qualified, sorted names. Returns empty when the file is missing or
/// carries none.
fn detect_legacy_keys(config_path: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(config_path) else {
        return Vec::new();
    };
    deprecated_keys_in_config(&content)
}

/// Write the complete serialized ruleset + schema files atomically.
fn write_complete_ruleset(
    jit_root: &Path,
    rules_path: &Path,
    serialized: &SerializedRuleSet,
) -> Result<()> {
    write_schema_files(jit_root, &serialized.schema_files)?;
    write_atomic(rules_path, &serialized.rules_toml)
}

/// Append any serialized default rule whose name is not already present in the
/// existing user `rules.toml` (coexistence, D5). Returns the names skipped
/// because the user file already defined them.
///
/// To re-render the appended rules deterministically WITHOUT re-implementing the
/// renderer, we parse the freshly-serialized complete file, drop the rules whose
/// names already exist, and re-serialize the remainder, appending its
/// `[[rules]]` blocks (header stripped) to the user file. Schema files are
/// written only for the appended rules.
fn append_missing_rules(
    jit_root: &Path,
    rules_path: &Path,
    serialized: &SerializedRuleSet,
) -> Result<Vec<String>> {
    let existing = std::fs::read_to_string(rules_path)
        .with_context(|| format!("reading {}", rules_path.display()))?;
    let existing_set = RuleSet::from_toml_str(&existing, jit_root)
        .with_context(|| format!("parsing existing {}", rules_path.display()))?;
    let existing_names: std::collections::HashSet<&str> =
        existing_set.rules.iter().map(|r| r.name.as_str()).collect();

    // Parse the complete serialized set (its schema files are not yet on disk,
    // so JsonSchema rules would fail to load; instead, work from the rendered
    // text and the rule NAMES via the in-code default set is overkill — parse the
    // body by splitting on `[[rules]]` blocks, keyed by their `name = "..."`).
    let blocks = split_rule_blocks(&serialized.rules_toml);

    let mut appended_text = String::new();
    let mut appended_schema_stems: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut skipped: Vec<String> = Vec::new();

    for block in &blocks {
        match block.name.as_deref() {
            Some(name) if existing_names.contains(name) => skipped.push(name.to_string()),
            Some(_) => {
                appended_text.push_str(&block.text);
                appended_text.push('\n');
                // A json-schema reference in this block needs its file written.
                if let Some(stem) = block.schema_stem() {
                    appended_schema_stems.insert(stem);
                }
            }
            None => {} // a malformed block with no name is not appended
        }
    }

    if !appended_text.is_empty() {
        let needed: Vec<SchemaFile> = serialized
            .schema_files
            .iter()
            .filter(|f| {
                let stem = f.name.strip_suffix(".json").unwrap_or(&f.name);
                appended_schema_stems.contains(stem)
            })
            .cloned()
            .collect();
        write_schema_files(jit_root, &needed)?;

        let mut merged = existing;
        if !merged.ends_with('\n') {
            merged.push('\n');
        }
        merged.push('\n');
        merged.push_str(MIGRATION_APPEND_HEADER);
        merged.push_str(&appended_text);
        write_atomic(rules_path, &merged)?;
    }

    skipped.sort();
    Ok(skipped)
}

const MIGRATION_APPEND_HEADER: &str =
    "# --- appended by `jit init` migration (legacy config keys) ---\n\n";

/// One `[[rules]]` block extracted from rendered `rules.toml` text.
struct RuleBlock {
    /// The block text (including the `[[rules]]` line, no trailing blank line).
    text: String,
    /// The parsed `name = "..."` value, if present.
    name: Option<String>,
}

impl RuleBlock {
    /// The schema file stem referenced by a `json-schema = "schemas/<stem>.json"`
    /// line in this block, if any.
    fn schema_stem(&self) -> Option<String> {
        let line = self
            .text
            .lines()
            .find(|l| l.trim_start().starts_with("assert") && l.contains("json-schema"))?;
        let start = line.find("schemas/")? + "schemas/".len();
        let rest = &line[start..];
        let end = rest.find(".json")?;
        Some(rest[..end].to_string())
    }
}

/// Split rendered `rules.toml` text (no graph tables span `[[rules]]` lines, the
/// renderer is one rule per block separated by a blank line) into per-rule
/// blocks, extracting each `name`.
fn split_rule_blocks(text: &str) -> Vec<RuleBlock> {
    let mut blocks: Vec<RuleBlock> = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    let flush = |lines: &mut Vec<&str>, blocks: &mut Vec<RuleBlock>| {
        if lines.is_empty() {
            return;
        }
        let text = lines.join("\n");
        let name = lines
            .iter()
            .find_map(|l| parse_name_line(l))
            .map(|s| s.to_string());
        blocks.push(RuleBlock { text, name });
        lines.clear();
    };

    for line in text.lines() {
        if line.starts_with("[[rules]]") {
            flush(&mut current, &mut blocks);
            current.push(line);
        } else if !current.is_empty() {
            current.push(line);
        }
    }
    flush(&mut current, &mut blocks);

    // Trim trailing blank lines from each block's text.
    for block in &mut blocks {
        while block.text.ends_with('\n') || block.text.ends_with(' ') {
            block.text.pop();
        }
    }
    blocks
}

/// Parse a `name = "value"` line into the unquoted name.
fn parse_name_line(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix("name")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    let inner = rest.strip_prefix('"')?;
    inner.strip_suffix('"')
}

/// Write schema files under `<jit_root>/schemas/`, creating the directory.
fn write_schema_files(jit_root: &Path, files: &[SchemaFile]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let schemas_dir = jit_root.join("schemas");
    std::fs::create_dir_all(&schemas_dir)
        .with_context(|| format!("creating {}", schemas_dir.display()))?;
    for file in files {
        write_atomic(&schemas_dir.join(&file.name), &file.content)?;
    }
    Ok(())
}

/// Strip the given fully-qualified deprecated keys from `config.toml` in place,
/// preserving other content, comments, and formatting (toml_edit). A missing key
/// is ignored (idempotent). When stripping empties the `[validation]` table, the
/// now-bare table header is REMOVED too (the superseded branch did not do this).
/// Registry `[namespaces.<ns>]` tables are kept (their `description`/`unique`
/// keys stay live). The rewrite is atomic (temp + rename).
fn strip_keys_from_config(config_path: &Path, keys: &[String]) -> Result<()> {
    if !config_path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parsing {} for migration", config_path.display()))?;

    for key in keys {
        let parts: Vec<&str> = key.split('.').collect();
        match parts.as_slice() {
            ["validation", field] => {
                if let Some(table) = doc.get_mut("validation").and_then(|v| v.as_table_mut()) {
                    table.remove(field);
                }
            }
            ["namespaces", ns, field] => {
                if let Some(ns_table) = doc
                    .get_mut("namespaces")
                    .and_then(|v| v.as_table_mut())
                    .and_then(|t| t.get_mut(ns))
                    .and_then(|v| v.as_table_mut())
                {
                    ns_table.remove(field);
                }
            }
            _ => {}
        }
    }

    // Remove an emptied `[validation]` table header (D5/R8).
    if let Some(table) = doc.get("validation").and_then(|v| v.as_table()) {
        if table.is_empty() {
            doc.remove("validation");
        }
    }

    write_atomic(config_path, &doc.to_string())
}

/// Write `content` to `path` atomically (temp file + rename).
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_manager::ConfigManager;

    fn config_from_toml(toml: &str) -> JitConfig {
        toml::from_str(toml).expect("config parses")
    }

    fn namespaces_for(config: &JitConfig) -> LabelNamespaces {
        ConfigManager::new(".jit").namespaces_from_config(config)
    }

    /// A `.jit` dir with the given config.toml content.
    fn jit_with_config(content: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), content).unwrap();
        dir
    }

    /// The in-code intended defaults used for the fresh-scaffold path. For
    /// legacy/coexistence/already-migrated cases these are unused, but the API
    /// requires a value.
    fn fresh_defaults() -> (JitConfig, LabelNamespaces) {
        let config =
            crate::hierarchy_templates::HierarchyTemplate::default().intended_default_config();
        let ns = namespaces_for(&config);
        (config, ns)
    }

    #[test]
    fn test_serialize_complete_ruleset_reloads_with_all_default_rules() {
        let config = config_from_toml(
            r#"
[validation]
require_type_label = true

[namespaces.type]
description = "Issue type"
unique = true
values = ["task", "bug"]
required = true
"#,
        );
        let namespaces = namespaces_for(&config);
        let serialized = serialize_complete_ruleset(&config, &namespaces);

        let dir = tempfile::tempdir().unwrap();
        write_schema_files(dir.path(), &serialized.schema_files).unwrap();
        std::fs::write(dir.path().join("rules.toml"), &serialized.rules_toml).unwrap();
        let set = RuleSet::load(dir.path()).expect("complete rules.toml must reload");

        // Every default rule is present (no subset split).
        let names: std::collections::HashSet<&str> =
            set.rules.iter().map(|r| r.name.as_str()).collect();
        for expected in [
            "default:require-type-label",
            "default:label-format",
            "default:namespace-registry",
            "default:type-hierarchy-known",
            "default:namespace-values:type",
            "default:namespace-unique:type",
            "default:namespace-required:type",
            "default:orphan-leaf",
            "default:strategic-consistency",
        ] {
            assert!(names.contains(expected), "missing rule: {expected}");
        }
    }

    #[test]
    fn test_migrate_legacy_repo_writes_complete_file_and_strips_keys() {
        let dir = jit_with_config(
            r#"# header comment
[validation]
default_type = "task"
strictness = "loose"
require_type_label = true
label_regex = '^team:[a-z]+$'
reject_malformed_labels = true
enforce_namespace_registry = true
warn_orphaned_leaves = true

[namespaces.type]
description = "Issue type"
unique = true
values = ["task", "bug"]
required = true
"#,
        );
        let config =
            config_from_toml(&std::fs::read_to_string(dir.path().join("config.toml")).unwrap());
        let namespaces = namespaces_for(&config);
        let outcome =
            migrate_or_scaffold(dir.path(), &config, &namespaces, &fresh_defaults()).unwrap();

        assert_eq!(outcome.state, MigrationState::LegacyMigrated);
        // The six enforcement keys + namespace constraints are stripped.
        for key in [
            "validation.require_type_label",
            "validation.label_regex",
            "validation.reject_malformed_labels",
            "validation.enforce_namespace_registry",
            "validation.warn_orphaned_leaves",
            "namespaces.type.values",
            "namespaces.type.required",
        ] {
            assert!(
                outcome.stripped_keys.contains(&key.to_string()),
                "expected {key} stripped, got {:?}",
                outcome.stripped_keys
            );
        }

        let rewritten = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        // Live keys retained.
        assert!(rewritten.contains("default_type = \"task\""));
        assert!(rewritten.contains("strictness = \"loose\""));
        assert!(rewritten.contains("# header comment"));
        assert!(rewritten.contains("description = \"Issue type\""));
        assert!(rewritten.contains("unique = true"));
        // Stripped keys gone.
        assert!(!rewritten.contains("require_type_label"));
        assert!(!rewritten.contains("label_regex"));
        assert!(!rewritten.contains("reject_malformed_labels"));

        // The written rules.toml reloads with the complete default set.
        let set = RuleSet::load(dir.path()).unwrap();
        assert!(set.rules.iter().any(|r| r.name == "default:label-format"));
        assert!(set
            .rules
            .iter()
            .any(|r| r.name == "default:label-format-custom"));
    }

    #[test]
    fn test_strip_removes_emptied_validation_header() {
        // A [validation] table that becomes empty after stripping loses its header.
        let dir = jit_with_config(
            r#"[validation]
require_type_label = true
label_regex = '^x'

[namespaces.type]
description = "t"
unique = true
"#,
        );
        let config_path = dir.path().join("config.toml");
        strip_keys_from_config(
            &config_path,
            &[
                "validation.require_type_label".to_string(),
                "validation.label_regex".to_string(),
            ],
        )
        .unwrap();
        let rewritten = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            !rewritten.contains("[validation]"),
            "emptied [validation] header must be removed: {rewritten}"
        );
        // The namespace table survives.
        assert!(rewritten.contains("[namespaces.type]"));
    }

    #[test]
    fn test_strip_keeps_nonempty_validation_header() {
        let dir = jit_with_config(
            r#"[validation]
default_type = "task"
require_type_label = true
"#,
        );
        let config_path = dir.path().join("config.toml");
        strip_keys_from_config(&config_path, &["validation.require_type_label".to_string()])
            .unwrap();
        let rewritten = std::fs::read_to_string(&config_path).unwrap();
        assert!(rewritten.contains("[validation]"));
        assert!(rewritten.contains("default_type = \"task\""));
        assert!(!rewritten.contains("require_type_label"));
    }

    #[test]
    fn test_migrate_is_idempotent() {
        let dir = jit_with_config(
            r#"[validation]
require_type_label = true

[namespaces.type]
description = "Issue type"
unique = true
"#,
        );
        let config =
            config_from_toml(&std::fs::read_to_string(dir.path().join("config.toml")).unwrap());
        let namespaces = namespaces_for(&config);

        let first =
            migrate_or_scaffold(dir.path(), &config, &namespaces, &fresh_defaults()).unwrap();
        assert_eq!(first.state, MigrationState::LegacyMigrated);
        assert!(!first.stripped_keys.is_empty());

        // Reload the (now-stripped) config and re-run: a TRUE no-op (file present,
        // no legacy keys) — NOT the coexistence/append path.
        let config2 =
            config_from_toml(&std::fs::read_to_string(dir.path().join("config.toml")).unwrap());
        let namespaces2 = namespaces_for(&config2);
        let rules_before = std::fs::read_to_string(dir.path().join("rules.toml")).unwrap();
        let second =
            migrate_or_scaffold(dir.path(), &config2, &namespaces2, &fresh_defaults()).unwrap();
        assert_eq!(
            second.state,
            MigrationState::AlreadyMigrated,
            "second init must be a true no-op"
        );
        assert!(second.is_noop());
        assert!(second.stripped_keys.is_empty(), "no keys left to strip");
        assert!(
            second.skipped_existing.is_empty(),
            "no-op must not emit skipped-rule warnings"
        );
        // The file is unchanged.
        let rules_after = std::fs::read_to_string(dir.path().join("rules.toml")).unwrap();
        assert_eq!(
            rules_before, rules_after,
            "idempotent: rules.toml unchanged"
        );
    }

    #[test]
    fn test_fresh_scaffold_uses_in_code_defaults_not_clean_on_disk_config() {
        // A repo whose on-disk config is already CLEAN (post-migration shape: no
        // enforcement keys, no namespace constraints) and has no rules.toml is a
        // FRESH scaffold: the complete ruleset comes from the in-code intended
        // defaults, so it still reproduces the rich checks. No keys stripped.
        let dir = jit_with_config(
            r#"[validation]
strictness = "loose"
default_type = "task"

[namespaces.type]
description = "Issue type"
unique = true
"#,
        );
        let config =
            config_from_toml(&std::fs::read_to_string(dir.path().join("config.toml")).unwrap());
        let namespaces = namespaces_for(&config);
        let outcome =
            migrate_or_scaffold(dir.path(), &config, &namespaces, &fresh_defaults()).unwrap();

        assert_eq!(outcome.state, MigrationState::FreshScaffold);
        assert!(outcome.stripped_keys.is_empty());
        assert!(outcome.skipped_existing.is_empty());

        // The scaffolded rules.toml carries the RICH constraints from the in-code
        // defaults, NOT derived from the (constraint-free) on-disk config.
        let set = RuleSet::load(dir.path()).unwrap();
        assert!(set
            .rules
            .iter()
            .any(|r| r.name == "default:namespace-values:type"));
        assert!(set
            .rules
            .iter()
            .any(|r| r.name == "default:namespace-required:type"));
        assert!(set
            .rules
            .iter()
            .any(|r| r.name == "default:namespace-pattern:milestone"));
        // The on-disk config stays clean (nothing added, nothing stripped).
        let config_after = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!config_after.contains("values ="));
        assert!(!config_after.contains("reject_malformed_labels"));
    }

    #[test]
    fn test_coexistence_appends_by_name_without_clobber() {
        // A user rules.toml already exists with a custom rule AND one default rule
        // name; legacy keys remain. Migration appends the missing defaults, skips
        // the already-present name, preserves the user rule, and strips the keys.
        let dir = jit_with_config(
            r#"[validation]
require_type_label = true

[namespaces.type]
description = "Issue type"
unique = true
"#,
        );
        std::fs::write(
            dir.path().join("rules.toml"),
            r#"[[rules]]
name = "user:custom"
when = { type = "epic" }
assert = { require-section = { heading = "Goals" } }

[[rules]]
name = "default:label-format"
severity = "error"
enforce = true
assert = { require-section = { heading = "PLACEHOLDER" } }
"#,
        )
        .unwrap();

        let config =
            config_from_toml(&std::fs::read_to_string(dir.path().join("config.toml")).unwrap());
        let namespaces = namespaces_for(&config);
        let outcome =
            migrate_or_scaffold(dir.path(), &config, &namespaces, &fresh_defaults()).unwrap();

        assert_eq!(outcome.state, MigrationState::Coexistence);
        // The pre-existing default name is skipped (not clobbered).
        assert!(outcome
            .skipped_existing
            .contains(&"default:label-format".to_string()));
        // Keys are still stripped.
        assert!(outcome
            .stripped_keys
            .contains(&"validation.require_type_label".to_string()));

        let set = RuleSet::load(dir.path()).unwrap();
        // User rule preserved.
        assert!(set.rules.iter().any(|r| r.name == "user:custom"));
        // The user's (placeholder) default:label-format is preserved, NOT replaced.
        let lf: Vec<_> = set
            .rules
            .iter()
            .filter(|r| r.name == "default:label-format")
            .collect();
        assert_eq!(lf.len(), 1, "no duplicate default:label-format");
        // A default that was NOT present is appended.
        assert!(set
            .rules
            .iter()
            .any(|r| r.name == "default:require-type-label"));
    }
}
