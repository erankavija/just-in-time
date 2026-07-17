//! Persistence for the validation ruleset files (`rules.toml` + `schemas/*.json`).
//!
//! The validation layer ([`crate::validation::serialize`]) produces the CONTENT
//! (the `rules.toml` body and the schema file name/content pairs); this storage
//! module owns the on-disk `rules.toml` + `schemas/` layout and performs the
//! writes, so those storage paths live only in the storage layer. All writes go
//! through the shared atomic writer ([`crate::storage::atomic_write`]),
//! preserving the temp-file + rename invariant.

use crate::storage::atomic_write::write_file_atomic;
use crate::validation::rules::DEFAULT_ORIGIN;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// The operative validation ruleset file, relative to the `.jit` root.
const RULES_FILE: &str = "rules.toml";
/// The directory (relative to the `.jit` root) holding rule-referenced schemas.
const SCHEMAS_DIR: &str = "schemas";

/// Whether this repository already has a materialized validation ruleset
/// (`<jit_root>/rules.toml`).
///
/// `jit init` scaffolds the default ruleset only when this is `false`, so a
/// present, user-edited `rules.toml` is never clobbered.
pub fn has_validation_ruleset(jit_root: &Path) -> bool {
    jit_root.join(RULES_FILE).exists()
}

/// Persist a serialized validation ruleset: `rules.toml` plus the
/// `schemas/<name>.json` files it references, each written atomically (temp
/// file + rename in the target's directory).
///
/// `schema_files` is a list of `(file_name, content)` pairs produced by the
/// validation serializer; this function owns where they land (`schemas/`).
/// Writing is the LAST step so a reader that observes `rules.toml` also sees the
/// schema files it references.
pub fn write_validation_ruleset(
    jit_root: &Path,
    rules_toml: &str,
    schema_files: &[(String, String)],
) -> Result<()> {
    if !schema_files.is_empty() {
        let schemas_dir = jit_root.join(SCHEMAS_DIR);
        std::fs::create_dir_all(&schemas_dir)
            .with_context(|| format!("creating {}", schemas_dir.display()))?;
        for (name, content) in schema_files {
            write_file_atomic(&schemas_dir.join(name), content)?;
        }
    }
    write_file_atomic(&jit_root.join(RULES_FILE), rules_toml)
}

/// Regenerate the single baked schema file `<jit_root>/schemas/<file_name>` from
/// `content`, but only when a materialized `schemas/` layout already exists.
///
/// A `rules.toml`-less repo builds its schemas in memory (the read path) and has
/// no on-disk `schemas/` to keep in sync, so this is a no-op there: it writes
/// only when the target file OR the `schemas/` directory already exists. Returns
/// `true` when a file was written. Idempotent (rewriting current content yields
/// identical bytes) and atomic (temp + rename).
pub fn write_baked_schema(jit_root: &Path, file_name: &str, content: &str) -> Result<bool> {
    let schemas_dir = jit_root.join(SCHEMAS_DIR);
    let target = schemas_dir.join(file_name);
    // Only refresh a materialized layout: if neither the file nor its directory
    // exists, the repo has no baked schemas (read path builds them in memory), so
    // there is nothing to keep in sync.
    if !target.exists() && !schemas_dir.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(&schemas_dir)
        .with_context(|| format!("creating {}", schemas_dir.display()))?;
    write_file_atomic(&target, content)?;
    Ok(true)
}

/// Rewrite the leading header region of `<jit_root>/rules.toml` to `header`,
/// preserving every `[[rules]]` block below it (and any comments authored inside
/// them) verbatim.
///
/// The header region is the leading trivia before the first `[[rules]]` table —
/// a generated comment block, modelled as the prefix decoration of the first
/// `rules` entry in a [`toml_edit::DocumentMut`]. Republishing it (setting that
/// prefix) always states the current default-rule contract without disturbing
/// any rule body, so a `[[rules]]` sequence inside a comment or a multiline rule
/// description can never be mistaken for the first table. A ruleset file with no
/// `[[rules]]` block (an intentionally empty ruleset) is rewritten to `header`
/// alone. A no-op when `rules.toml` is absent (the scaffold path writes a fresh
/// file, header included) or already current. Returns `true` when the file was
/// rewritten. Atomic (temp + rename).
pub fn rewrite_rules_header(jit_root: &Path, header: &str) -> Result<bool> {
    let path = jit_root.join(RULES_FILE);
    if !path.exists() {
        return Ok(false);
    }
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parsing {} as TOML", path.display()))?;
    let rebuilt = match doc
        .get_mut(RULES_ARRAY_KEY)
        .and_then(toml_edit::Item::as_array_of_tables_mut)
        .and_then(|rules| rules.get_mut(0))
    {
        Some(first) => {
            first.decor_mut().set_prefix(header);
            doc.to_string()
        }
        // No `[[rules]]` block: the file is header-only, replaced wholesale.
        None => header.to_string(),
    };
    if rebuilt == content {
        return Ok(false);
    }
    write_file_atomic(&path, &rebuilt)?;
    Ok(true)
}

/// Minimal per-rule identity read off a `[[rules]]` block: just enough
/// (`name`, `origin`) to compute the membership diff that drives
/// [`sync_namespace_unique_rules`], without pulling in the full `assert`-table
/// deserialization [`crate::validation::rules::RuleSet`] performs (which
/// resolves schema files and is unnecessary — and unnecessarily fragile — for a
/// membership sync that never inspects a rule's assertion).
#[derive(Debug, Deserialize)]
struct RuleIdentity {
    name: String,
    #[serde(default)]
    origin: Option<String>,
}

/// Top-level shape of `rules.toml` for [`RuleIdentity`] extraction.
#[derive(Debug, Default, Deserialize)]
struct RuleIdentitiesFile {
    #[serde(default)]
    rules: Vec<RuleIdentity>,
}

/// Read every rule's `(name, origin)` identity from `<jit_root>/rules.toml`.
///
/// Identity-only parsing: assertion tables are never deserialized and schema
/// references never resolved, so this succeeds on a file whose full
/// [`RuleSet`](crate::validation::rules::RuleSet) load would fail on a custom
/// rule — the membership write-through must not be strandable by an unrelated
/// rule's defect (jit:d74a9ed1 review F1). Returns an empty list when the file
/// is absent.
pub fn read_rule_identities(jit_root: &Path) -> Result<Vec<(String, Option<String>)>> {
    let path = jit_root.join(RULES_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let identities: RuleIdentitiesFile = toml::from_str(&content)
        .with_context(|| format!("Failed to parse rule identities from {}", path.display()))?;
    Ok(identities
        .rules
        .into_iter()
        .map(|r| (r.name, r.origin))
        .collect())
}

/// The `rules.toml` array-of-tables key holding every `[[rules]]` block.
const RULES_ARRAY_KEY: &str = "rules";

/// Apply a `namespace-unique-*` DEFAULT-rule membership delta to
/// `<jit_root>/rules.toml`: append each pre-rendered `[[rules]]` block in
/// `to_add` at the END of the file, and remove the `origin = "default"` block
/// for each name in `to_drop`.
///
/// The file is edited through a [`toml_edit::DocumentMut`], which is lossless
/// for untouched content: every OTHER byte of the file — other rules' fields,
/// hand-edited policy fields on surviving default rules, custom rules, comments,
/// blank-line formatting, exotic-but-valid header spellings, multiline strings,
/// and any UNRELATED trailing tables — round-trips byte-exact (REQ-01,
/// jit:d74a9ed1). The drop matches the document model's `name`/`origin` values
/// directly, so it can never over-reach into neighbouring or trailing content.
/// Dropping the FIRST rule transfers its prefix decoration — where `toml_edit`
/// stores the file's leading header/comments — onto the new first rule, so the
/// leading trivia is preserved rather than removed with the entry. Dropping the
/// last remaining rule leaves the generated header to the header-rewrite path
/// while any unrelated trailing tables survive.
///
/// `to_add` entries are typically rendered via
/// [`crate::validation::serialize::render_rule_block`] and are appended verbatim
/// (exactly the canonical text `jit init` scaffolds), so blocks jit itself wrote
/// stay byte-stable. A name in `to_drop` that does not match an
/// `origin = "default"` block (already absent, or present only under a different
/// origin) is silently skipped — dropping something not there is a no-op.
///
/// A no-op (`Ok(false)`) when `rules.toml` is absent (nothing to sync), when
/// neither list changes the file, or when the diff is empty. Atomic (temp +
/// rename). Returns an error only if the file does not parse as TOML.
pub fn sync_namespace_unique_rules(
    jit_root: &Path,
    to_add: &[String],
    to_drop: &[String],
) -> Result<bool> {
    let path = jit_root.join(RULES_FILE);
    if !path.exists() {
        return Ok(false);
    }
    if to_add.is_empty() && to_drop.is_empty() {
        return Ok(false);
    }

    let content =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parsing {} as TOML", path.display()))?;

    // DROP through the document model: remove each `origin = "default"` rule
    // named in `to_drop`. Matching on the model's own `name`/`origin` values can
    // never over-reach into neighbouring or trailing content.
    let dropped_any = !to_drop.is_empty()
        && doc
            .get_mut(RULES_ARRAY_KEY)
            .and_then(toml_edit::Item::as_array_of_tables_mut)
            .is_some_and(|rules| {
                let is_dropped = |table: &toml_edit::Table| {
                    table.get("origin").and_then(toml_edit::Item::as_str) == Some(DEFAULT_ORIGIN)
                        && table
                            .get("name")
                            .and_then(toml_edit::Item::as_str)
                            .is_some_and(|name| to_drop.iter().any(|d| d == name))
                };
                // `toml_edit` stores the file's leading header/comments as the
                // FIRST entry's prefix decoration. If that entry is dropped, capture
                // its prefix so it can be transferred onto the new first entry —
                // otherwise the leading trivia would vanish with the removed entry.
                let leading_prefix = rules
                    .get(0)
                    .filter(|first| is_dropped(first))
                    .and_then(|first| first.decor().prefix())
                    .and_then(|raw| raw.as_str())
                    .map(str::to_owned);

                let before = rules.len();
                rules.retain(|table| !is_dropped(table));

                // Restore the leading trivia onto whatever is now first (mirrors the
                // first-entry prefix handling in `rewrite_rules_header`). When the
                // drop empties the array there is no entry to carry it; the generated
                // header is re-published by the header-rewrite path instead.
                if let (Some(prefix), Some(new_first)) = (leading_prefix, rules.get_mut(0)) {
                    new_first.decor_mut().set_prefix(prefix);
                }
                rules.len() != before
            });

    // Re-serialize only when a drop actually removed a table: `toml_edit` is
    // lossless for retained content EXCEPT that it canonicalizes exotic-but-valid
    // header spellings (`[[ rules ]]`, quoted keys). When nothing was dropped
    // (an add-only sync, or a drop of an absent name), keep the original bytes so
    // hand-authored formatting is never rewritten just to append a rule.
    let mut rebuilt = if dropped_any {
        doc.to_string()
    } else {
        content.clone()
    };
    // ADD by appending each already-rendered canonical block at EOF — the same
    // text `serialize_ruleset` concatenates, so an appended rule is byte-identical
    // to a scaffolded one. Guarantee exactly one separating newline: a source file
    // (or serialized document) may lack a final newline, and appending a block
    // onto an unterminated last line would fuse the tokens.
    if !to_add.is_empty() && !rebuilt.is_empty() && !rebuilt.ends_with('\n') {
        rebuilt.push('\n');
    }
    for block in to_add {
        rebuilt.push_str(block);
    }

    if rebuilt == content {
        return Ok(false);
    }
    write_file_atomic(&path, &rebuilt)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{LabelNamespace, LabelNamespaces};
    use crate::validation::defaults::{default_ruleset, TYPE_HIERARCHY_SCHEMA_FILE};
    use crate::validation::serialize::{serialize_ruleset, type_hierarchy_schema_content};
    use std::collections::HashMap;

    fn registry(entries: Vec<(&str, LabelNamespace)>) -> LabelNamespaces {
        let mut namespaces = HashMap::new();
        for (name, ns) in entries {
            namespaces.insert(name.to_string(), ns);
        }
        LabelNamespaces {
            schema_version: 2,
            namespaces,
            type_hierarchy: None,
            label_associations: None,
            strategic_types: None,
        }
    }

    /// Build a registry carrying an explicit type hierarchy.
    fn registry_with_hierarchy(types: &[(&str, u8)]) -> LabelNamespaces {
        let mut hierarchy = HashMap::new();
        for (name, level) in types {
            hierarchy.insert(name.to_string(), *level);
        }
        LabelNamespaces {
            schema_version: 2,
            namespaces: HashMap::new(),
            type_hierarchy: Some(hierarchy),
            label_associations: None,
            strategic_types: None,
        }
    }

    /// Materialize the default ruleset to `jit_root` (the `jit init` scaffold:
    /// validation produces content, storage persists it).
    fn scaffold(jit_root: &Path, reg: &LabelNamespaces) {
        let serialized = serialize_ruleset(&default_ruleset(reg));
        let schema_files: Vec<(String, String)> = serialized
            .schema_files
            .into_iter()
            .map(|f| (f.name, f.content))
            .collect();
        write_validation_ruleset(jit_root, &serialized.rules_toml, &schema_files).unwrap();
    }

    /// Regenerate the baked type-hierarchy schema from `reg`.
    fn regenerate(jit_root: &Path, reg: &LabelNamespaces) -> bool {
        write_baked_schema(
            jit_root,
            TYPE_HIERARCHY_SCHEMA_FILE,
            &type_hierarchy_schema_content(reg),
        )
        .unwrap()
    }

    #[test]
    fn test_regenerate_type_hierarchy_schema_writes_declared_types() {
        // After `jit init`, the baked schema exists. Regenerating from a hierarchy
        // that declares a NEW type must include that type in the enum.
        let dir = tempfile::tempdir().unwrap();
        scaffold(dir.path(), &registry(vec![]));
        let target = dir.path().join("schemas").join(TYPE_HIERARCHY_SCHEMA_FILE);
        assert!(target.exists(), "scaffold writes the baked schema");

        let reg = registry_with_hierarchy(&[("epic", 2), ("planning", 3), ("task", 4)]);
        assert!(regenerate(dir.path(), &reg), "existing layout is refreshed");

        let content = std::fs::read_to_string(&target).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        let enum_values = &json["properties"]["labels"]["properties"]["type"]["items"]["enum"];
        let names: Vec<&str> = enum_values
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(
            names.contains(&"planning"),
            "declared type present: {names:?}"
        );
        assert!(names.contains(&"epic"));
        assert!(names.contains(&"task"));
    }

    #[test]
    fn test_regenerate_type_hierarchy_schema_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        scaffold(dir.path(), &registry(vec![]));
        let reg = registry_with_hierarchy(&[("epic", 2), ("task", 4)]);
        regenerate(dir.path(), &reg);
        let target = dir.path().join("schemas").join(TYPE_HIERARCHY_SCHEMA_FILE);
        let first = std::fs::read_to_string(&target).unwrap();
        regenerate(dir.path(), &reg);
        let second = std::fs::read_to_string(&target).unwrap();
        assert_eq!(first, second, "regeneration is byte-identical (idempotent)");
    }

    #[test]
    fn test_regenerate_is_noop_without_baked_schemas() {
        // A repo with no `schemas/` dir (read path builds rules in memory) has
        // nothing to refresh: the regenerator is a no-op and creates no file.
        let dir = tempfile::tempdir().unwrap();
        let reg = registry_with_hierarchy(&[("epic", 2)]);
        assert!(!regenerate(dir.path(), &reg), "no baked layout => no write");
        assert!(!dir.path().join("schemas").exists());
    }

    #[test]
    fn test_regenerated_schema_matches_scaffolded_default() {
        // The regenerator and the `jit init` scaffold must produce the SAME baked
        // type-hierarchy schema for the same hierarchy (one source, not two: R5).
        let reg = registry_with_hierarchy(&[("epic", 2), ("planning", 3), ("task", 4)]);

        let scaffold_dir = tempfile::tempdir().unwrap();
        scaffold(scaffold_dir.path(), &reg);
        let scaffolded = std::fs::read_to_string(
            scaffold_dir
                .path()
                .join("schemas")
                .join(TYPE_HIERARCHY_SCHEMA_FILE),
        )
        .unwrap();

        let regen_dir = tempfile::tempdir().unwrap();
        // Seed an empty-default baked layout, then regenerate from the same hierarchy.
        scaffold(regen_dir.path(), &registry(vec![]));
        regenerate(regen_dir.path(), &reg);
        let regenerated = std::fs::read_to_string(
            regen_dir
                .path()
                .join("schemas")
                .join(TYPE_HIERARCHY_SCHEMA_FILE),
        )
        .unwrap();

        assert_eq!(scaffolded, regenerated, "scaffold and regen must agree");
    }

    #[test]
    fn test_rewrite_rules_header_replaces_header_and_keeps_bodies() {
        // The leading comment block is replaced with the new header; every
        // `[[rules]]` block (and its inline comments) is preserved byte-for-byte.
        let dir = tempfile::tempdir().unwrap();
        let original = "# old header line 1\n# old header line 2\n\n\
             [[rules]]\nname = \"keep-me\"\n# a custom comment\nassert = { require-section = { heading = \"Goals\" } }\n";
        std::fs::write(dir.path().join(RULES_FILE), original).unwrap();

        let new_header = "# new header\n# second line\n\n";
        let wrote = rewrite_rules_header(dir.path(), new_header).unwrap();
        assert!(wrote, "a differing header must be rewritten");

        let updated = std::fs::read_to_string(dir.path().join(RULES_FILE)).unwrap();
        assert!(updated.starts_with(new_header), "new header is on top");
        assert!(!updated.contains("old header"), "old header is gone");
        assert!(
            updated.contains("name = \"keep-me\"") && updated.contains("# a custom comment"),
            "rule bodies and their comments are preserved: {updated}"
        );
    }

    #[test]
    fn test_rewrite_rules_header_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let header = "# header\n\n";
        std::fs::write(
            dir.path().join(RULES_FILE),
            format!("{header}[[rules]]\nname = \"r\"\nassert = {{ require-section = {{ heading = \"H\" }} }}\n"),
        )
        .unwrap();
        assert!(
            !rewrite_rules_header(dir.path(), header).unwrap(),
            "an already-current header is a no-op"
        );
    }

    #[test]
    fn test_rewrite_rules_header_noop_without_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            !rewrite_rules_header(dir.path(), "# header\n\n").unwrap(),
            "absent rules.toml => nothing to rewrite"
        );
        assert!(!dir.path().join(RULES_FILE).exists());
    }

    // -- sync_namespace_unique_rules (jit:d74a9ed1) ---------------------------

    /// A minimal, hand-authored `rules.toml`: one default rule, one custom rule
    /// carrying its own comment, used to exercise byte-exact preservation.
    const HAND_AUTHORED_RULES: &str = "\
# a hand-authored header, left untouched by membership sync\n\
\n\
[[rules]]\n\
name = \"label-format\"\n\
origin = \"default\"\n\
severity = \"error\"\n\
enforce = true\n\
assert = { require-section = { heading = \"unused-in-this-test\" } }\n\
\n\
[[rules]]\n\
name = \"namespace-unique-team\"\n\
origin = \"default\"\n\
severity = \"warn\"\n\
enforce = false\n\
assert = { require-label = { label = \"team:*\", min = 0, max = 1 } }\n\
\n\
[[rules]]\n\
name = \"custom-shape\"\n\
# a hand-authored comment on a custom rule\n\
severity = \"warn\"\n\
assert = { require-section = { heading = \"Goals\" } }\n\
";

    fn write_rules(dir: &Path, content: &str) {
        std::fs::write(dir.join(RULES_FILE), content).unwrap();
    }

    fn read_rules(dir: &Path) -> String {
        std::fs::read_to_string(dir.join(RULES_FILE)).unwrap()
    }

    #[test]
    fn test_sync_appends_new_block_preserving_rest_byte_exact() {
        let dir = tempfile::tempdir().unwrap();
        write_rules(dir.path(), HAND_AUTHORED_RULES);

        let new_block = "[[rules]]\nname = \"namespace-unique-squad\"\norigin = \"default\"\nseverity = \"error\"\nenforce = true\nassert = { require-label = { label = \"squad:*\", min = 0, max = 1 } }\n\n";
        let changed =
            sync_namespace_unique_rules(dir.path(), &[new_block.to_string()], &[]).unwrap();
        assert!(changed);

        let updated = read_rules(dir.path());
        assert!(
            updated.starts_with(HAND_AUTHORED_RULES),
            "every original byte survives as a prefix, new block appended after:\n{updated}"
        );
        assert_eq!(updated, format!("{HAND_AUTHORED_RULES}{new_block}"));
    }

    #[test]
    fn test_sync_drops_default_block_preserving_rest_byte_exact() {
        let dir = tempfile::tempdir().unwrap();
        write_rules(dir.path(), HAND_AUTHORED_RULES);

        let changed =
            sync_namespace_unique_rules(dir.path(), &[], &["namespace-unique-team".to_string()])
                .unwrap();
        assert!(changed);

        let updated = read_rules(dir.path());
        assert!(!updated.contains("namespace-unique-team"));
        // Every other rule survives verbatim, including the custom rule's
        // hand-authored comment and the header.
        assert!(updated.contains("# a hand-authored header, left untouched by membership sync"));
        assert!(updated.contains("name = \"label-format\""));
        assert!(updated.contains("name = \"custom-shape\""));
        assert!(updated.contains("# a hand-authored comment on a custom rule"));
        // Reconstructed from the surviving blocks: removing the middle block
        // leaves the first and third concatenated after the header.
        let expected = HAND_AUTHORED_RULES.replacen(
            "[[rules]]\nname = \"namespace-unique-team\"\norigin = \"default\"\nseverity = \"warn\"\nenforce = false\nassert = { require-label = { label = \"team:*\", min = 0, max = 1 } }\n\n",
            "",
            1,
        );
        assert_eq!(updated, expected);
    }

    #[test]
    fn test_sync_drop_preserves_unrelated_trailing_table() {
        // The dropped rule is the LAST `[[rules]]` block, followed by an unrelated
        // top-level table. The drop must remove only that rule and leave the
        // trailing table intact — a raw splitter whose final block ran to EOF
        // would delete it (jit:d74a9ed1 review F1, round 5).
        let dir = tempfile::tempdir().unwrap();
        let content = "\
[[rules]]\n\
name = \"label-format\"\n\
origin = \"default\"\n\
assert = { require-section = { heading = \"H\" } }\n\
\n\
[[rules]]\n\
name = \"namespace-unique-team\"\n\
origin = \"default\"\n\
assert = { require-label = { label = \"team:*\", min = 0, max = 1 } }\n\
\n\
[extra]\n\
note = \"unrelated trailing table, must survive\"\n\
";
        write_rules(dir.path(), content);

        let changed =
            sync_namespace_unique_rules(dir.path(), &[], &["namespace-unique-team".to_string()])
                .unwrap();
        assert!(changed);

        let updated = read_rules(dir.path());
        assert!(
            !updated.contains("namespace-unique-team"),
            "the final rules block is dropped"
        );
        assert!(
            updated.contains("name = \"label-format\""),
            "the first rule survives"
        );
        // The unrelated trailing table is NOT swallowed by dropping the final block.
        assert!(
            updated.contains("[extra]\nnote = \"unrelated trailing table, must survive\""),
            "trailing table preserved verbatim:\n{updated}"
        );
        // Exactly one `rules` entry remains; the trailing table is not a rule.
        assert_eq!(read_rule_identities(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn test_sync_drop_of_first_rule_preserves_leading_header() {
        // The dropped rule is the FIRST `[[rules]]` entry. `toml_edit` stores the
        // file's leading header/comments as that entry's prefix decoration, so a
        // naive removal would delete them. The header must move onto the new first
        // entry byte-exact (jit:d74a9ed1 review F1, round 6).
        let dir = tempfile::tempdir().unwrap();
        let content = "# LEADING HEADER\n# line two\n\n[[rules]]\nname = \"namespace-unique-team\"\norigin = \"default\"\nassert = { require-label = { label = \"team:*\", min = 0, max = 1 } }\n\n[[rules]]\nname = \"custom-rule\"\nseverity = \"warn\"\nassert = { require-section = { heading = \"Goals\" } }\n";
        write_rules(dir.path(), content);

        let changed =
            sync_namespace_unique_rules(dir.path(), &[], &["namespace-unique-team".to_string()])
                .unwrap();
        assert!(changed);

        let updated = read_rules(dir.path());
        // The leading comment block survives, now atop the surviving rule.
        let expected = "# LEADING HEADER\n# line two\n\n[[rules]]\nname = \"custom-rule\"\nseverity = \"warn\"\nassert = { require-section = { heading = \"Goals\" } }\n";
        assert_eq!(updated, expected, "leading header transferred byte-exact");
        assert_eq!(read_rule_identities(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn test_sync_drop_of_all_rules_preserves_trailing_table() {
        // Dropping the ONLY rule empties the array. No entry remains to carry the
        // generated leading header (the header-rewrite path republishes it), but an
        // unrelated trailing table must not be lost (jit:d74a9ed1 review, round 6b).
        let dir = tempfile::tempdir().unwrap();
        let content = "# HEADER\n\n[[rules]]\nname = \"namespace-unique-team\"\norigin = \"default\"\nassert = { require-label = { label = \"team:*\", min = 0, max = 1 } }\n\n[extra]\nnote = \"survives\"\n";
        write_rules(dir.path(), content);

        let changed =
            sync_namespace_unique_rules(dir.path(), &[], &["namespace-unique-team".to_string()])
                .unwrap();
        assert!(changed);

        let updated = read_rules(dir.path());
        assert!(
            !updated.contains("namespace-unique-team"),
            "the only rule is dropped"
        );
        assert!(
            updated.contains("[extra]\nnote = \"survives\""),
            "unrelated trailing table preserved:\n{updated}"
        );
        assert_eq!(read_rule_identities(dir.path()).unwrap().len(), 0);
    }

    #[test]
    fn test_sync_ignores_drop_name_under_non_default_origin() {
        // A custom rule happens to be NAMED like a namespace-unique row (no
        // `origin = "default"`). It must never be dropped by this function, even
        // if its name appears in `to_drop`.
        let dir = tempfile::tempdir().unwrap();
        let content = "\
[[rules]]\n\
name = \"namespace-unique-team\"\n\
severity = \"warn\"\n\
assert = { require-section = { heading = \"Goals\" } }\n\
";
        write_rules(dir.path(), content);

        let changed =
            sync_namespace_unique_rules(dir.path(), &[], &["namespace-unique-team".to_string()])
                .unwrap();
        assert!(!changed, "a non-default row is never dropped");
        assert_eq!(read_rules(dir.path()), content);
    }

    #[test]
    fn test_sync_add_and_drop_together() {
        let dir = tempfile::tempdir().unwrap();
        write_rules(dir.path(), HAND_AUTHORED_RULES);

        let new_block = "[[rules]]\nname = \"namespace-unique-squad\"\norigin = \"default\"\nseverity = \"error\"\nenforce = true\nassert = { require-label = { label = \"squad:*\", min = 0, max = 1 } }\n\n";
        let changed = sync_namespace_unique_rules(
            dir.path(),
            &[new_block.to_string()],
            &["namespace-unique-team".to_string()],
        )
        .unwrap();
        assert!(changed);

        let updated = read_rules(dir.path());
        assert!(!updated.contains("namespace-unique-team"));
        assert!(updated.contains("namespace-unique-squad"));
        assert!(updated.contains("name = \"custom-shape\""));
    }

    #[test]
    fn test_sync_is_noop_without_file() {
        let dir = tempfile::tempdir().unwrap();
        let changed = sync_namespace_unique_rules(
            dir.path(),
            &[
                "[[rules]]\nname = \"x\"\nassert = { require-section = { heading = \"H\" } }\n\n"
                    .to_string(),
            ],
            &[],
        )
        .unwrap();
        assert!(!changed);
        assert!(!dir.path().join(RULES_FILE).exists());
    }

    #[test]
    fn test_sync_is_noop_with_empty_diff() {
        let dir = tempfile::tempdir().unwrap();
        write_rules(dir.path(), HAND_AUTHORED_RULES);
        let changed = sync_namespace_unique_rules(dir.path(), &[], &[]).unwrap();
        assert!(!changed);
        assert_eq!(read_rules(dir.path()), HAND_AUTHORED_RULES);
    }

    #[test]
    fn test_sync_drop_of_absent_name_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        write_rules(dir.path(), HAND_AUTHORED_RULES);
        let changed = sync_namespace_unique_rules(
            dir.path(),
            &[],
            &["namespace-unique-nonexistent".to_string()],
        )
        .unwrap();
        assert!(!changed);
        assert_eq!(read_rules(dir.path()), HAND_AUTHORED_RULES);
    }

    #[test]
    fn test_sync_appends_to_file_without_trailing_newline() {
        // A valid, hand-authored rules.toml whose final line has NO trailing
        // newline. Appending a generated block naively would fuse `} }[[rules]]`
        // into one line, corrupting the TOML (jit:d74a9ed1 review F1).
        let dir = tempfile::tempdir().unwrap();
        let no_newline = "\
[[rules]]\n\
name = \"label-format\"\n\
origin = \"default\"\n\
assert = { require-section = { heading = \"H\" } }";
        assert!(
            !no_newline.ends_with('\n'),
            "fixture must lack a final newline"
        );
        write_rules(dir.path(), no_newline);

        let new_block = "[[rules]]\nname = \"namespace-unique-squad\"\norigin = \"default\"\nassert = { require-label = { label = \"squad:*\", min = 0, max = 1 } }\n";
        let changed =
            sync_namespace_unique_rules(dir.path(), &[new_block.to_string()], &[]).unwrap();
        assert!(changed);

        let updated = read_rules(dir.path());
        // Exactly one newline separates the old last token from the new block —
        // added, not doubled — so the file parses cleanly.
        assert!(
            updated.contains("heading = \"H\" } }\n[[rules]]\nname = \"namespace-unique-squad\""),
            "one separating newline inserted:\n{updated}"
        );
        // The round-trip parses and carries both the old and the appended rule.
        let names: Vec<String> = read_rule_identities(dir.path())
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(names, vec!["label-format", "namespace-unique-squad"]);
    }

    #[test]
    fn test_sync_preserves_multiline_string_containing_rules_marker() {
        // A custom rule whose `description` multiline string contains a line
        // reading `[[rules]]`. A naive splitter would treat that inner line as a
        // table boundary, count one block too many, and abort the sync — leaving
        // rules.toml stale (jit:d74a9ed1 review F2).
        let dir = tempfile::tempdir().unwrap();
        let content = "\
# header\n\
\n\
[[rules]]\n\
name = \"label-format\"\n\
origin = \"default\"\n\
assert = { require-section = { heading = \"H\" } }\n\
\n\
[[rules]]\n\
name = \"custom-doc\"\n\
description = '''\n\
Documents the on-disk shape:\n\
[[rules]]\n\
name = \"...\"\n\
This is prose inside a string, not a real table.\n\
'''\n\
assert = { require-section = { heading = \"Goals\" } }\n\
";
        write_rules(dir.path(), content);
        // Sanity: the file really parses to exactly two rules.
        assert_eq!(read_rule_identities(dir.path()).unwrap().len(), 2);

        let new_block = "[[rules]]\nname = \"namespace-unique-squad\"\norigin = \"default\"\nassert = { require-label = { label = \"squad:*\", min = 0, max = 1 } }\n\n";
        let changed =
            sync_namespace_unique_rules(dir.path(), &[new_block.to_string()], &[]).unwrap();
        assert!(
            changed,
            "sync proceeds despite the inner `[[rules]]` marker"
        );

        let updated = read_rules(dir.path());
        // The custom rule and its multiline description survive byte-exact, the
        // membership add landed, and the whole file still parses (no corruption).
        assert!(
            updated.starts_with(content),
            "original preserved as a prefix"
        );
        assert!(updated.contains("This is prose inside a string, not a real table."));
        assert!(updated.contains("name = \"namespace-unique-squad\""));
        assert_eq!(read_rule_identities(dir.path()).unwrap().len(), 3);
    }

    #[test]
    fn test_sync_preserves_indented_rules_header() {
        // TOML permits leading whitespace before a table header, so an indented
        // `[[rules]]` is a valid custom rule. A column-zero-only splitter would
        // miss it, undercount blocks against the parsed rule count, and abort the
        // sync — leaving rules.toml stale (jit:d74a9ed1 review F1, round 3).
        let dir = tempfile::tempdir().unwrap();
        // NB: the indented block is one segment with explicit `\n  ` so Rust's
        // `\`-continuation does not strip the leading spaces we are testing.
        let content = "\
# header\n\
\n\
[[rules]]\n\
name = \"label-format\"\n\
origin = \"default\"\n\
assert = { require-section = { heading = \"H\" } }\n\
\n  [[rules]]\n  name = \"custom-indented\"\n  severity = \"warn\"\n  assert = { require-section = { heading = \"Goals\" } }\n";
        write_rules(dir.path(), content);
        // Sanity: the indented header is a real, parseable table (two rules).
        assert_eq!(read_rule_identities(dir.path()).unwrap().len(), 2);

        let new_block = "[[rules]]\nname = \"namespace-unique-squad\"\norigin = \"default\"\nassert = { require-label = { label = \"squad:*\", min = 0, max = 1 } }\n\n";
        let changed =
            sync_namespace_unique_rules(dir.path(), &[new_block.to_string()], &[]).unwrap();
        assert!(changed, "sync proceeds despite the indented header");

        let updated = read_rules(dir.path());
        // The indented rule survives byte-exact (indentation included), the
        // membership add landed, and the whole file still parses (no corruption).
        assert!(
            updated.starts_with(content),
            "original preserved as a prefix"
        );
        assert!(updated.contains("  [[rules]]\n  name = \"custom-indented\""));
        assert!(updated.contains("name = \"namespace-unique-squad\""));
        assert_eq!(read_rule_identities(dir.path()).unwrap().len(), 3);
    }

    /// Build a rules.toml with one column-zero `[[rules]]` default rule and a
    /// SECOND `rules` entry whose header is `variant_header` (an alternate but
    /// equivalent spelling of the `rules` array-of-tables header), then run a
    /// membership add. Asserts the sync recognized BOTH entries (no block-count
    /// abort), preserved the variant byte-exact, and appended the new rule.
    fn assert_rules_header_variant_synced(variant_header: &str) {
        let dir = tempfile::tempdir().unwrap();
        let content = format!(
            "[[rules]]\n\
name = \"label-format\"\n\
origin = \"default\"\n\
assert = {{ require-section = {{ heading = \"H\" }} }}\n\
\n\
{variant_header}\n\
name = \"custom-variant\"\n\
severity = \"warn\"\n\
assert = {{ require-section = {{ heading = \"Goals\" }} }}\n"
        );
        write_rules(dir.path(), &content);
        assert_eq!(
            read_rule_identities(dir.path()).unwrap().len(),
            2,
            "variant header `{variant_header}` is a valid second rules entry"
        );

        let new_block = "[[rules]]\nname = \"namespace-unique-squad\"\norigin = \"default\"\nassert = { require-label = { label = \"squad:*\", min = 0, max = 1 } }\n\n";
        let changed =
            sync_namespace_unique_rules(dir.path(), &[new_block.to_string()], &[]).unwrap();
        assert!(
            changed,
            "sync recognizes variant `{variant_header}` and proceeds"
        );

        let updated = read_rules(dir.path());
        assert!(
            updated.starts_with(&content),
            "variant `{variant_header}` preserved byte-exact:\n{updated}"
        );
        assert!(updated.contains(variant_header), "variant header intact");
        assert!(updated.contains("name = \"namespace-unique-squad\""));
        assert_eq!(read_rule_identities(dir.path()).unwrap().len(), 3);
    }

    #[test]
    fn test_sync_recognizes_internal_whitespace_rules_header() {
        assert_rules_header_variant_synced("[[ rules ]]");
    }

    #[test]
    fn test_sync_recognizes_tabbed_rules_header() {
        assert_rules_header_variant_synced("[[\trules\t]]");
    }

    #[test]
    fn test_sync_recognizes_basic_quoted_rules_header() {
        assert_rules_header_variant_synced("[[\"rules\"]]");
    }

    #[test]
    fn test_sync_recognizes_literal_quoted_rules_header() {
        assert_rules_header_variant_synced("[['rules']]");
    }

    #[test]
    fn test_sync_recognizes_trailing_comment_rules_header() {
        assert_rules_header_variant_synced("[[rules]] # a trailing note");
    }

    #[test]
    fn test_sync_ignores_non_rules_array_header() {
        // `[[ruleset]]` is a DIFFERENT table path, not a `rules` entry. It must
        // NOT count as a rules block (which would desync the block/identity
        // counts and abort) and must survive the sync untouched.
        let dir = tempfile::tempdir().unwrap();
        let content = "\
[[rules]]\n\
name = \"label-format\"\n\
origin = \"default\"\n\
assert = { require-section = { heading = \"H\" } }\n\
\n\
[[ruleset]]\n\
name = \"not-a-rule\"\n\
";
        write_rules(dir.path(), content);
        // Only ONE `rules` entry; `[[ruleset]]` is a separate table.
        assert_eq!(read_rule_identities(dir.path()).unwrap().len(), 1);

        let new_block = "[[rules]]\nname = \"namespace-unique-squad\"\norigin = \"default\"\nassert = { require-label = { label = \"squad:*\", min = 0, max = 1 } }\n\n";
        let changed =
            sync_namespace_unique_rules(dir.path(), &[new_block.to_string()], &[]).unwrap();
        assert!(changed);

        let updated = read_rules(dir.path());
        assert!(
            updated.starts_with(content),
            "the `[[ruleset]]` custom table survives untouched:\n{updated}"
        );
        assert!(updated.contains("[[ruleset]]\nname = \"not-a-rule\""));
        assert!(updated.contains("name = \"namespace-unique-squad\""));
        // The file's one `rules` entry plus the appended one.
        assert_eq!(read_rule_identities(dir.path()).unwrap().len(), 2);
    }

    #[test]
    fn test_sync_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        write_rules(dir.path(), HAND_AUTHORED_RULES);
        let new_block = "[[rules]]\nname = \"namespace-unique-squad\"\norigin = \"default\"\nseverity = \"error\"\nenforce = true\nassert = { require-label = { label = \"squad:*\", min = 0, max = 1 } }\n\n";

        sync_namespace_unique_rules(dir.path(), &[new_block.to_string()], &[]).unwrap();
        let once = read_rules(dir.path());

        // Re-applying the SAME add against the now-updated file would duplicate
        // the block (the caller is responsible for only passing a fresh diff);
        // this test instead confirms a truly empty second diff changes nothing.
        let changed = sync_namespace_unique_rules(dir.path(), &[], &[]).unwrap();
        assert!(!changed);
        assert_eq!(read_rules(dir.path()), once);
    }
}
