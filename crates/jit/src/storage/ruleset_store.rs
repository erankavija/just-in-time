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
/// The header region is everything before the first line that starts a `[[rules]]`
/// table — a generated comment block, republished so it always states the current
/// default-rule contract without disturbing custom rules. A ruleset file with no
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
    // The body starts at the first REAL `[[rules]]` table header. `rule_header_offsets`
    // resolves those string-/comment-aware, so a `[[rules]]` sequence inside a header
    // comment or a multiline rule description is never mistaken for the first table.
    let body_start = rule_header_offsets(&content).first().copied();
    let rebuilt = match body_start {
        Some(idx) => format!("{header}{}", &content[idx..]),
        None => header.to_string(),
    };
    if rebuilt == content {
        return Ok(false);
    }
    write_file_atomic(&path, &rebuilt)?;
    Ok(true)
}

/// Minimal per-rule identity read off a `[[rules]]` block: just enough
/// (`name`, `origin`) to drive [`sync_namespace_unique_rules`]'s structural
/// add/drop, without pulling in the full `assert`-table deserialization
/// [`crate::validation::rules::RuleSet`] performs (which resolves schema
/// files and is unnecessary — and unnecessarily fragile — for a membership
/// sync that never inspects a rule's assertion).
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

/// Byte offsets of the LINE START of every REAL `[[rules]]` table header, in
/// file order.
///
/// "Real" means the header is preceded on its physical line ONLY by whitespace
/// (TOML permits leading indentation before a table header) AND is outside any
/// string or comment context. A `[[rules]]` sequence inside a `#` comment or a
/// `'''`/`"""` multiline string (a rule's `description`, say) is prose, not a
/// table boundary, so it is skipped. The scan tracks TOML string/comment state
/// across lines, so it cannot be fooled by an in-string occurrence that a naive
/// `\n[[rules]]` search would mis-split on (jit:d74a9ed1 review F2).
///
/// The recorded offset is the header line's first byte — its indentation
/// included — so a block sliced from one offset to the next carries the header
/// line verbatim (jit:d74a9ed1 review F1, round 3).
///
/// Manual state tracking (rather than `toml_edit`) keeps every consumer working
/// on raw byte slices of the original file, preserving each block's exact
/// bytes — comments, blank lines, and field formatting — which the membership
/// sync's byte-exact preservation contract depends on.
fn rule_header_offsets(content: &str) -> Vec<usize> {
    enum Mode {
        Normal,
        Comment,
        BasicSingle,
        LiteralSingle,
        BasicMulti,
        LiteralMulti,
    }

    let bytes = content.as_bytes();
    let len = content.len();
    // Advance past one whole char at `idx` (keeps `i` on a UTF-8 boundary).
    let char_len = |idx: usize| content[idx..].chars().next().map_or(1, char::len_utf8);

    let mut offsets = Vec::new();
    let mut mode = Mode::Normal;
    // `line_begin` is the current physical line's first byte; `leading_ws` is true
    // while, in `Normal` mode, only whitespace has been seen since `line_begin`.
    // A `[[rules]]` reached with `leading_ws` still set is a real header, and its
    // recorded offset is `line_begin` (indentation included).
    let mut line_begin = 0;
    let mut leading_ws = true;
    let mut i = 0;

    while i < len {
        match mode {
            Mode::Normal => {
                if content[i..].starts_with("\"\"\"") {
                    mode = Mode::BasicMulti;
                    leading_ws = false;
                    i += 3;
                    continue;
                }
                if content[i..].starts_with("'''") {
                    mode = Mode::LiteralMulti;
                    leading_ws = false;
                    i += 3;
                    continue;
                }
                if leading_ws && content[i..].starts_with("[[rules]]") {
                    offsets.push(line_begin);
                }
                let c = bytes[i];
                match c {
                    b'#' => mode = Mode::Comment,
                    b'"' => mode = Mode::BasicSingle,
                    b'\'' => mode = Mode::LiteralSingle,
                    _ => {}
                }
                if c == b'\n' {
                    line_begin = i + 1;
                    leading_ws = true;
                } else if c != b' ' && c != b'\t' {
                    leading_ws = false;
                }
                i += char_len(i);
            }
            Mode::Comment => {
                let c = bytes[i];
                if c == b'\n' {
                    mode = Mode::Normal;
                    line_begin = i + 1;
                    leading_ws = true;
                }
                i += char_len(i);
            }
            Mode::BasicSingle => {
                let c = bytes[i];
                if c == b'\\' {
                    // Skip the escaped char so `\"` does not close the string.
                    i += 1;
                    if i < len {
                        i += char_len(i);
                    }
                    continue;
                }
                if c == b'\n' {
                    // Malformed (a single-line string cannot span lines); recover.
                    mode = Mode::Normal;
                    line_begin = i + 1;
                    leading_ws = true;
                } else if c == b'"' {
                    mode = Mode::Normal;
                }
                i += char_len(i);
            }
            Mode::LiteralSingle => {
                let c = bytes[i];
                if c == b'\n' {
                    mode = Mode::Normal;
                    line_begin = i + 1;
                    leading_ws = true;
                } else if c == b'\'' {
                    mode = Mode::Normal;
                }
                i += char_len(i);
            }
            Mode::BasicMulti => {
                if bytes[i] == b'\\' {
                    // Escapes apply in basic strings, so `\"""` is not a close.
                    i += 1;
                    if i < len {
                        i += char_len(i);
                    }
                    continue;
                }
                if content[i..].starts_with("\"\"\"") {
                    mode = Mode::Normal;
                    i += 3;
                    continue;
                }
                i += char_len(i);
            }
            Mode::LiteralMulti => {
                if content[i..].starts_with("'''") {
                    mode = Mode::Normal;
                    i += 3;
                    continue;
                }
                i += char_len(i);
            }
        }
    }

    offsets
}

/// Split `rules.toml` content into its leading header (everything before the
/// first `[[rules]]` table) and the raw text of each `[[rules]]` block, in
/// file order.
///
/// A block's text runs from its `[[rules]]` line up to (but not including) the
/// next `[[rules]]` line, or EOF — so it carries every line that belongs to
/// it: its fields AND any comments/blank lines authored inside or immediately
/// after it. Table boundaries come from [`rule_header_offsets`], so a
/// `[[rules]]`-looking string inside a description or comment is never mistaken
/// for one.
fn split_rule_blocks(content: &str) -> (&str, Vec<&str>) {
    let starts = rule_header_offsets(content);

    let header = match starts.first() {
        Some(&first) => &content[..first],
        None => content,
    };
    let blocks = starts
        .iter()
        .enumerate()
        .map(|(i, &start)| {
            let end = starts.get(i + 1).copied().unwrap_or(content.len());
            &content[start..end]
        })
        .collect();
    (header, blocks)
}

/// Apply a `namespace-unique-*` DEFAULT-rule membership delta to
/// `<jit_root>/rules.toml`: append each pre-rendered `[[rules]]` block in
/// `to_add` at the END of the file, and remove the `origin = "default"` block
/// for each name in `to_drop` — identified STRUCTURALLY (by parsing each
/// block's own `name`/`origin`, never by content-diffing), so every OTHER
/// byte of the file — every other rule's fields, hand-edited policy fields on
/// surviving default rules, custom rules, comments, and blank-line formatting
/// — survives untouched (REQ-01, jit:d74a9ed1).
///
/// `to_add` entries are typically rendered via
/// [`crate::validation::serialize::render_rule_block`]. A name in `to_drop`
/// that does not match an `origin = "default"` block (already absent, or
/// present only under a different origin) is silently skipped — dropping
/// something not there is a no-op, not an error.
///
/// A no-op (`Ok(false)`) when `rules.toml` is absent (nothing to sync), or
/// when neither list changes the file. Atomic (temp + rename). Returns an
/// error if the file's `[[rules]]` blocks cannot be parsed to identity, or if
/// the parsed rule count does not match the number of `[[rules]]` blocks found
/// (a malformed or unexpectedly-shaped file this function cannot safely edit
/// structurally).
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
    let (header, blocks) = split_rule_blocks(&content);

    let identities: RuleIdentitiesFile = toml::from_str(&content)
        .with_context(|| format!("parsing {} to locate rule identities", path.display()))?;
    if identities.rules.len() != blocks.len() {
        anyhow::bail!(
            "cannot structurally sync {}: parsed {} rule(s) but found {} `[[rules]]` block(s)",
            path.display(),
            identities.rules.len(),
            blocks.len()
        );
    }

    let mut rebuilt = header.to_string();
    for (identity, block) in identities.rules.iter().zip(blocks.iter()) {
        let drop = identity.origin.as_deref() == Some(DEFAULT_ORIGIN)
            && to_drop.iter().any(|name| name == &identity.name);
        if !drop {
            rebuilt.push_str(block);
        }
    }
    // A hand-authored rules.toml may end without a final newline; appending a
    // generated block directly would fuse its `[[rules]]` line onto the file's
    // last token, corrupting the TOML. Guarantee exactly one separating newline:
    // add it only when content precedes the append and lacks a trailing newline.
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
