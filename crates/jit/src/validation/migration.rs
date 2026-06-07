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
//! - **Brand-new repo** (`config.toml` did NOT pre-exist; no `rules.toml`, no
//!   legacy keys): scaffold the complete serialized ruleset from the in-code RICH
//!   intended defaults (the opinionated starter ruleset). No strip, no migration
//!   message.
//! - **Existing repo, clean config** (`config.toml` already present, no
//!   `rules.toml`, NO legacy keys): MATERIALIZE current behavior — serialize the
//!   complete ruleset from the repo's ACTUAL config, byte-equivalent to
//!   `effective_rules`'s absent-file fallback, so `jit init` strengthens nothing.
//!   No strip. (Distinguished from a brand-new repo via the caller's
//!   `config_already_existed` signal; conflating the two SILENTLY changed
//!   validation behavior — the bug fixed here.)
//! - **Legacy repo, no `rules.toml`, legacy keys present:** write the complete
//!   serialized ruleset from the LIVE config + schema files, then strip the
//!   migrated keys from `config.toml` (removing an emptied `[validation]` header).
//! - **Coexistence** (`rules.toml` already exists AND legacy keys remain): never
//!   clobber the user file — APPEND each default rule whose name is not already
//!   present (skipping, with a warning, any whose schema file would overwrite a
//!   differing user-authored `.jit/schemas/*.json`), then strip the legacy keys.
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

/// Which of the repo states [`migrate_or_scaffold`] acted on (D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationState {
    /// BRAND-NEW repo (`config.toml` did NOT pre-exist — `jit init` just created
    /// it from the template), no `rules.toml`, no legacy keys: a complete
    /// `rules.toml` was scaffolded from the in-code RICH intended defaults (the
    /// opinionated starter ruleset). No strip, no migration message.
    FreshScaffold,
    /// EXISTING repo (`config.toml` already present), no `rules.toml`, no legacy
    /// keys: MATERIALIZED current behavior — serialized the complete ruleset from
    /// the repo's ACTUAL live config (bare defaults), byte-equivalent to what
    /// `effective_rules`'s absent-file fallback already produces. Nothing
    /// observable changes; the file is merely materialized. No strip.
    MaterializeCurrent,
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
    /// Default rule names skipped during coexistence because appending them would
    /// have CLOBBERED a pre-existing, differing user-authored schema file under
    /// `.jit/schemas/` (empty otherwise). The rule is left out entirely so its
    /// reference always matches the file actually on disk.
    pub skipped_schema_collision: Vec<String>,
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
///   scaffold of a BRAND-NEW repo, where the on-disk config ships clean (post-
///   template) and the rich starter ruleset is the intended opinionated default.
/// - `config_already_existed` is the "did `config.toml` exist BEFORE `jit init`
///   touched anything?" signal recorded by the caller. It distinguishes a brand-
///   new repo (config did NOT pre-exist) from an existing repo whose config is
///   present but carries no legacy keys. For the latter, migration must PRESERVE
///   behavior — it materializes the repo's ACTUAL current effective ruleset
///   (bare defaults from the live config), NOT the rich starter defaults, so
///   `jit init` changes nothing observable except writing the file.
///
/// The states (keyed on `rules.toml` presence, legacy-key presence, and for the
/// clean/no-rules case the `config_already_existed` signal):
///
/// | rules.toml | legacy keys | config pre-existed | state              | action                          |
/// |------------|-------------|--------------------|--------------------|---------------------------------|
/// | absent     | none        | NO  (brand-new)    | FreshScaffold      | write complete file from `fresh_defaults` (rich) |
/// | absent     | none        | YES (existing)     | MaterializeCurrent | write complete file from the ACTUAL `config` (bare), no strip |
/// | absent     | present     | (either)           | LegacyMigrated     | write complete file from `config`, strip keys |
/// | present    | present     | (either)           | Coexistence        | append missing defaults by name, strip keys |
/// | present    | none        | (either)           | AlreadyMigrated    | NO-OP (no append, no warnings, no strip) |
///
/// The AlreadyMigrated guard is what makes a repeated `jit init` a true no-op
/// (no spurious "skipped" warnings).
pub fn migrate_or_scaffold(
    jit_root: &Path,
    config: &JitConfig,
    namespaces: &LabelNamespaces,
    fresh_defaults: &(JitConfig, LabelNamespaces),
    config_already_existed: bool,
) -> Result<MigrationOutcome> {
    let rules_path = jit_root.join("rules.toml");
    let config_path = jit_root.join("config.toml");
    let file_present = rules_path.exists();
    let legacy_keys = detect_legacy_keys(&config_path);

    let (state, stripped_keys, skipped_existing, skipped_schema_collision) =
        match (file_present, legacy_keys.is_empty()) {
            // Already-migrated: a present file with no stale keys is a true no-op.
            (true, true) => (
                MigrationState::AlreadyMigrated,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),

            // Coexistence: preserve the user file, append missing defaults by name,
            // then strip the stale keys. The complete ruleset is derived from the
            // LIVE config (it still carries the legacy constraints).
            (true, false) => {
                let serialized = serialize_complete_ruleset(config, namespaces);
                let appended = append_missing_rules(jit_root, &rules_path, &serialized)?;
                strip_keys_from_config(&config_path, &legacy_keys)?;
                (
                    MigrationState::Coexistence,
                    legacy_keys,
                    appended.skipped_existing,
                    appended.skipped_schema_collision,
                )
            }

            // No rules.toml, no legacy keys: split on whether config pre-existed.
            (false, true) if !config_already_existed => {
                // BRAND-NEW repo: scaffold the RICH opinionated starter ruleset from
                // the in-code intended defaults. No keys to strip.
                let (fresh_config, fresh_ns) = fresh_defaults;
                let serialized = serialize_complete_ruleset(fresh_config, fresh_ns);
                write_complete_ruleset(jit_root, &rules_path, &serialized)?;
                (
                    MigrationState::FreshScaffold,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                )
            }
            (false, true) => {
                // EXISTING repo, clean config: MATERIALIZE current behavior.
                // Serialize the complete ruleset from the repo's ACTUAL config —
                // byte-equivalent to `effective_rules`'s absent-file fallback — so
                // init strengthens nothing. No keys to strip.
                let serialized = serialize_complete_ruleset(config, namespaces);
                write_complete_ruleset(jit_root, &rules_path, &serialized)?;
                (
                    MigrationState::MaterializeCurrent,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                )
            }

            // Legacy migration: write the complete file from the LIVE config (which
            // carries the keys), then strip them.
            (false, false) => {
                let serialized = serialize_complete_ruleset(config, namespaces);
                write_complete_ruleset(jit_root, &rules_path, &serialized)?;
                strip_keys_from_config(&config_path, &legacy_keys)?;
                (
                    MigrationState::LegacyMigrated,
                    legacy_keys,
                    Vec::new(),
                    Vec::new(),
                )
            }
        };

    Ok(MigrationOutcome {
        state,
        stripped_keys,
        skipped_existing,
        skipped_schema_collision,
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

/// The rules skipped while appending defaults in coexistence mode (D5).
struct AppendOutcome {
    /// Default rule names skipped because the user file already defined them.
    skipped_existing: Vec<String>,
    /// Default rule names skipped because appending them would have clobbered a
    /// pre-existing, differing user schema file under `.jit/schemas/`.
    skipped_schema_collision: Vec<String>,
}

/// Append any serialized default rule whose name is not already present in the
/// existing user `rules.toml` (coexistence, D5).
///
/// To re-render the appended rules deterministically WITHOUT re-implementing the
/// renderer, we parse the freshly-serialized complete file, drop the rules whose
/// names already exist, and re-serialize the remainder, appending its
/// `[[rules]]` blocks (header stripped) to the user file. Schema files are
/// written only for the appended rules.
///
/// Schema-file CLOBBER GUARD (Defect 2): a default rule's schema file name is
/// deterministic from its rule name, so it can collide with a pre-existing
/// user-authored `.jit/schemas/*.json` referenced by one of the user's own
/// rules. We MUST NOT overwrite such a file. If the target path already exists
/// with DIFFERENT content, the whole rule is SKIPPED (not appended) and reported
/// separately — leaving the rule out keeps its `json-schema` reference consistent
/// with whatever file is actually on disk. An IDENTICAL pre-existing file is fine
/// to reuse (no clobber, append the rule).
fn append_missing_rules(
    jit_root: &Path,
    rules_path: &Path,
    serialized: &SerializedRuleSet,
) -> Result<AppendOutcome> {
    let existing = std::fs::read_to_string(rules_path)
        .with_context(|| format!("reading {}", rules_path.display()))?;
    let existing_set = RuleSet::from_toml_str(&existing, jit_root)
        .with_context(|| format!("parsing existing {}", rules_path.display()))?;
    let existing_names: std::collections::HashSet<&str> =
        existing_set.rules.iter().map(|r| r.name.as_str()).collect();

    // Map schema stem -> the content we would write, so we can compare against any
    // pre-existing on-disk file before appending the referencing rule.
    let schema_by_stem: std::collections::HashMap<&str, &str> = serialized
        .schema_files
        .iter()
        .map(|f| {
            let stem = f.name.strip_suffix(".json").unwrap_or(&f.name);
            (stem, f.content.as_str())
        })
        .collect();
    let schemas_dir = jit_root.join("schemas");

    // Parse the complete serialized set (its schema files are not yet on disk,
    // so JsonSchema rules would fail to load; instead, work from the rendered
    // text and the rule NAMES via the in-code default set is overkill — parse the
    // body by splitting on `[[rules]]` blocks, keyed by their `name = "..."`).
    let blocks = split_rule_blocks(&serialized.rules_toml);

    let mut appended_text = String::new();
    let mut appended_schema_stems: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut skipped_existing: Vec<String> = Vec::new();
    let mut skipped_schema_collision: Vec<String> = Vec::new();

    for block in &blocks {
        let Some(name) = block.name.as_deref() else {
            continue; // a malformed block with no name is not appended
        };
        if existing_names.contains(name) {
            skipped_existing.push(name.to_string());
            continue;
        }
        // If this rule references a schema file that already exists on disk with
        // DIFFERENT content, skip the whole rule rather than clobber it.
        if let Some(stem) = block.schema_stem() {
            let target = schemas_dir.join(format!("{stem}.json"));
            if let Ok(on_disk) = std::fs::read_to_string(&target) {
                let would_write = schema_by_stem.get(stem.as_str()).copied().unwrap_or("");
                if on_disk != would_write {
                    skipped_schema_collision.push(name.to_string());
                    continue;
                }
            }
            appended_schema_stems.insert(stem);
        }
        appended_text.push_str(&block.text);
        appended_text.push('\n');
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

    skipped_existing.sort();
    skipped_schema_collision.sort();
    Ok(AppendOutcome {
        skipped_existing,
        skipped_schema_collision,
    })
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
            migrate_or_scaffold(dir.path(), &config, &namespaces, &fresh_defaults(), true).unwrap();

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
            migrate_or_scaffold(dir.path(), &config, &namespaces, &fresh_defaults(), true).unwrap();
        assert_eq!(first.state, MigrationState::LegacyMigrated);
        assert!(!first.stripped_keys.is_empty());

        // Reload the (now-stripped) config and re-run: a TRUE no-op (file present,
        // no legacy keys) — NOT the coexistence/append path.
        let config2 =
            config_from_toml(&std::fs::read_to_string(dir.path().join("config.toml")).unwrap());
        let namespaces2 = namespaces_for(&config2);
        let rules_before = std::fs::read_to_string(dir.path().join("rules.toml")).unwrap();
        let second =
            migrate_or_scaffold(dir.path(), &config2, &namespaces2, &fresh_defaults(), true)
                .unwrap();
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
    fn test_brand_new_repo_scaffolds_rich_in_code_defaults() {
        // A BRAND-NEW repo (config.toml did NOT pre-exist — passed as `false`) with
        // no rules.toml is a FreshScaffold: the complete ruleset comes from the
        // in-code RICH intended defaults (the opinionated starter ruleset), even
        // though the on-disk config the template wrote is constraint-free.
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
        // config_already_existed = false => brand-new repo.
        let outcome =
            migrate_or_scaffold(dir.path(), &config, &namespaces, &fresh_defaults(), false)
                .unwrap();

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
    fn test_existing_clean_repo_materializes_current_behavior_not_rich_defaults() {
        // Defect 1: an EXISTING repo (config.toml PRE-EXISTED — passed as `true`)
        // whose config is clean/loose and has no rules.toml and NO legacy keys must
        // MATERIALIZE its current behavior, NOT silently gain the rich starter
        // defaults. The written rules.toml must be byte-identical to serializing
        // the repo's ACTUAL config (= effective_rules' absent-file fallback).
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

        // What effective_rules() would produce from the ABSENT-file fallback for
        // this repo: default_ruleset(actual config, actual namespaces).
        let expected = serialize_complete_ruleset(&config, &namespaces);

        // config_already_existed = true => existing repo, must materialize current.
        let outcome =
            migrate_or_scaffold(dir.path(), &config, &namespaces, &fresh_defaults(), true).unwrap();

        assert_eq!(outcome.state, MigrationState::MaterializeCurrent);
        assert!(outcome.stripped_keys.is_empty());
        assert!(outcome.skipped_existing.is_empty());

        // The materialized rules.toml is BYTE-equivalent to the absent-file
        // fallback's ruleset — init strengthened nothing.
        let written = std::fs::read_to_string(dir.path().join("rules.toml")).unwrap();
        assert_eq!(
            written, expected.rules_toml,
            "materialized rules.toml must equal effective_rules' absent-file fallback"
        );

        // Concretely: the constraint-free config yields NO rich namespace
        // constraints (no values/required/pattern rules), unlike a fresh scaffold.
        let set = RuleSet::load(dir.path()).unwrap();
        assert!(
            !set.rules
                .iter()
                .any(|r| r.name == "default:namespace-values:type"),
            "clean existing repo must NOT gain namespace-values enforcement"
        );
        assert!(
            !set.rules
                .iter()
                .any(|r| r.name == "default:namespace-pattern:milestone"),
            "clean existing repo must NOT gain milestone-pattern enforcement"
        );
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
            migrate_or_scaffold(dir.path(), &config, &namespaces, &fresh_defaults(), true).unwrap();

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

    #[test]
    fn test_coexistence_does_not_clobber_user_schema_file() {
        // Defect 2: a user rules.toml + a user-authored schema file whose path
        // COLLIDES with the deterministic name a default rule would write
        // (`schemas/default-label-format.json`), but with DISTINCT content,
        // referenced by the user's own rule. The user does NOT define a rule named
        // `default:label-format`, so the default would normally be appended — and
        // appending it would overwrite the user's schema file. Migration must
        // instead SKIP that default rule, leave the user file untouched, preserve
        // the user's rules, and report the collision skip.
        let dir = jit_with_config(
            r#"[validation]
require_type_label = true

[namespaces.type]
description = "Issue type"
unique = true
"#,
        );

        // A user-authored schema file at the colliding path with DISTINCT content.
        let schemas_dir = dir.path().join("schemas");
        std::fs::create_dir_all(&schemas_dir).unwrap();
        let user_schema_path = schemas_dir.join("default-label-format.json");
        let user_schema = "{\n  \"type\": \"string\",\n  \"const\": \"USER-OWNED\"\n}\n";
        std::fs::write(&user_schema_path, user_schema).unwrap();

        // A user rule that REFERENCES that schema file (so it is user-owned), and
        // a custom rule. Note: NOT named `default:label-format`.
        std::fs::write(
            dir.path().join("rules.toml"),
            "[[rules]]\n\
             name = \"user:label-shape\"\n\
             severity = \"error\"\n\
             enforce = true\n\
             assert = { json-schema = \"schemas/default-label-format.json\" }\n\
             \n\
             [[rules]]\n\
             name = \"user:custom\"\n\
             when = { type = \"epic\" }\n\
             assert = { require-section = { heading = \"Goals\" } }\n",
        )
        .unwrap();

        let config =
            config_from_toml(&std::fs::read_to_string(dir.path().join("config.toml")).unwrap());
        let namespaces = namespaces_for(&config);
        let outcome =
            migrate_or_scaffold(dir.path(), &config, &namespaces, &fresh_defaults(), true).unwrap();

        assert_eq!(outcome.state, MigrationState::Coexistence);

        // The default:label-format rule is skipped due to the schema collision and
        // reported as such (a warning is emitted in the CLI path from this list).
        assert!(
            outcome
                .skipped_schema_collision
                .contains(&"default:label-format".to_string()),
            "default:label-format must be skipped to avoid clobbering, got {:?}",
            outcome.skipped_schema_collision
        );

        // The user's schema file is BYTE-for-byte unchanged.
        let after = std::fs::read_to_string(&user_schema_path).unwrap();
        assert_eq!(
            after, user_schema,
            "user-authored schema file must NOT be modified"
        );

        // Legacy keys are still stripped.
        assert!(outcome
            .stripped_keys
            .contains(&"validation.require_type_label".to_string()));

        // The user's rules are preserved; the clobbering default is NOT appended.
        let set = RuleSet::load(dir.path()).unwrap();
        assert!(set.rules.iter().any(|r| r.name == "user:label-shape"));
        assert!(set.rules.iter().any(|r| r.name == "user:custom"));
        assert!(
            !set.rules.iter().any(|r| r.name == "default:label-format"),
            "the colliding default must not be appended"
        );
        // A non-colliding default IS still appended (the guard is per-rule).
        assert!(set
            .rules
            .iter()
            .any(|r| r.name == "default:require-type-label"));
    }
}
