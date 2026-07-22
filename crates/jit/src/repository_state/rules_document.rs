//! Pure span-level `rules.toml` document edits (byte in, byte out).
//!
//! The default-rule family is a projection of the namespace registry, but the
//! file also carries hand-authored content — custom rules, comments, the header,
//! block order, and the editable policy fields (`severity`/`enforce`/selector) of
//! generated rules. These primitives edit ONLY the generated spans and preserve
//! every other byte: [`rewrite_header`] republishes the leading generated comment
//! region, and [`splice_default_membership`] appends newly generated
//! `namespace-unique-*` blocks and drops obsolete `origin = "default"` ones. Both
//! are pure `&str -> String` transforms consumed directly by the mutation derive
//! pipeline.

use crate::declarations::rules::DEFAULT_ORIGIN;
use anyhow::{Context, Result};
use serde::Deserialize;

/// Minimal per-rule identity read off a `[[rules]]` block: just enough
/// (`name`, `origin`) to compute the default-family membership diff, without the
/// full `assert`-table deserialization (which resolves schema files and is
/// unnecessarily fragile for a membership sync that never inspects assertions).
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

/// Read every rule's `(name, origin)` identity from `rules.toml` `content`.
///
/// Identity-only parsing: assertion tables are never deserialized and schema
/// references never resolved, so this succeeds on a file whose full
/// [`RuleSet`](crate::declarations::rules::RuleSet) load would fail on a custom
/// rule — the membership diff must not be strandable by an unrelated rule's defect.
pub fn parse_rule_identities(content: &str) -> Result<Vec<(String, Option<String>)>> {
    let identities: RuleIdentitiesFile =
        toml::from_str(content).context("parsing rule identities from rules.toml")?;
    Ok(identities
        .rules
        .into_iter()
        .map(|r| (r.name, r.origin))
        .collect())
}

/// The `rules.toml` array-of-tables key holding every `[[rules]]` block.
const RULES_ARRAY_KEY: &str = "rules";

/// Set the leading trivia (prefix decoration) of a top-level [`toml_edit::Item`]
/// to `prefix`, regardless of the item's shape. A table or bare value carries its
/// own decoration; an array-of-tables stores its leading trivia on its first
/// table. A `None` item (an unset key) has nowhere to hang trivia, so this is a
/// no-op there.
fn set_item_prefix(item: &mut toml_edit::Item, prefix: &str) {
    match item {
        toml_edit::Item::Table(table) => table.decor_mut().set_prefix(prefix),
        toml_edit::Item::Value(value) => value.decor_mut().set_prefix(prefix),
        toml_edit::Item::ArrayOfTables(tables) => {
            if let Some(first) = tables.get_mut(0) {
                first.decor_mut().set_prefix(prefix);
            }
        }
        toml_edit::Item::None => {}
    }
}

/// Read the leading trivia (prefix decoration) of a top-level [`toml_edit::Item`]
/// as an owned string, mirroring [`set_item_prefix`]'s shape handling. Returns
/// `None` when the item cannot carry a prefix (an empty array-of-tables or an
/// unset key) or when it carries none.
fn item_prefix(item: &toml_edit::Item) -> Option<String> {
    let decor = match item {
        toml_edit::Item::Table(table) => table.decor(),
        toml_edit::Item::Value(value) => value.decor(),
        toml_edit::Item::ArrayOfTables(tables) => tables.get(0)?.decor(),
        toml_edit::Item::None => return None,
    };
    decor
        .prefix()
        .and_then(|raw| raw.as_str())
        .map(str::to_owned)
}

/// The key of the first top-level item that is NOT the (possibly empty) `rules`
/// array, in document order. This is the file's TOP — where the leading
/// header/comments belong once no `[[rules]]` block remains to carry them.
fn first_non_rules_key(doc: &toml_edit::DocumentMut) -> Option<String> {
    doc.as_table()
        .iter()
        .find(|(key, _)| *key != RULES_ARRAY_KEY)
        .map(|(key, _)| key.to_owned())
}

/// The key of the top-level item immediately FOLLOWING the `rules` array in
/// document order, or `None` when `rules` is last (or absent). This is where the
/// emptied array's orphaned leading trivia is relocated.
fn key_after_rules(doc: &toml_edit::DocumentMut) -> Option<String> {
    doc.as_table()
        .iter()
        .skip_while(|(key, _)| *key != RULES_ARRAY_KEY)
        .nth(1)
        .map(|(key, _)| key.to_owned())
}

/// Rewrite the leading header region of `content` to `header`, preserving every
/// `[[rules]]` block below it (and any comments authored inside them) and all
/// other non-generated content.
///
/// The header region is the leading trivia before the first `[[rules]]` table,
/// modelled as the prefix decoration of the first `rules` entry. A ruleset with
/// no `[[rules]]` block but other content republishes the header onto the first
/// surviving top-level item's prefix, never by replacing the whole file. Only a
/// document with no top-level item at all becomes `header` alone. Returns the
/// rewritten bytes; the caller compares against `content` to decide whether to
/// persist.
pub fn rewrite_header(content: &str, header: &str) -> Result<String> {
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .context("parsing rules.toml as TOML")?;
    let rebuilt = match doc
        .get_mut(RULES_ARRAY_KEY)
        .and_then(toml_edit::Item::as_array_of_tables_mut)
        .and_then(|rules| rules.get_mut(0))
    {
        Some(first) => {
            first.decor_mut().set_prefix(header);
            doc.to_string()
        }
        None => match first_non_rules_key(&doc) {
            Some(key) => {
                if let Some(item) = doc.get_mut(&key) {
                    set_item_prefix(item, header);
                }
                doc.to_string()
            }
            None => header.to_string(),
        },
    };
    Ok(rebuilt)
}

/// Apply a `namespace-unique-*` DEFAULT-rule membership delta to `content`:
/// append each pre-rendered `[[rules]]` block in `to_add` at the END, and remove
/// the `origin = "default"` block for each name in `to_drop`.
///
/// The file is edited through a [`toml_edit::DocumentMut`], lossless for
/// untouched content. On an add-only edit every OTHER byte round-trips
/// byte-exact; a drop re-serializes the document (canonicalizing only
/// exotic-but-valid syntax, semantically lossless) and matches the model's own
/// `name`/`origin`, so it can never over-reach into neighbouring or trailing
/// content. Dropping the first rule transfers its leading trivia onto the new
/// first entry; dropping the last empties the array and relocates that trivia
/// onto the first surviving top-level item (or the document's trailing decor), so
/// the leading header/comments and any unrelated trailing tables all survive. A
/// `to_drop` name that matches no `origin = "default"` block is a no-op. Returns
/// the edited bytes; the caller compares against `content` to decide persistence.
pub fn splice_default_membership(
    content: &str,
    to_add: &[String],
    to_drop: &[String],
) -> Result<String> {
    if to_add.is_empty() && to_drop.is_empty() {
        return Ok(content.to_string());
    }
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .context("parsing rules.toml as TOML")?;

    let mut orphaned_leading_prefix: Option<String> = None;
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
                let leading_prefix = rules
                    .get(0)
                    .filter(|first| is_dropped(first))
                    .and_then(|first| first.decor().prefix())
                    .and_then(|raw| raw.as_str())
                    .map(str::to_owned);

                let before = rules.len();
                rules.retain(|table| !is_dropped(table));

                if let Some(prefix) = leading_prefix {
                    match rules.get_mut(0) {
                        Some(new_first) => new_first.decor_mut().set_prefix(prefix),
                        None => orphaned_leading_prefix = Some(prefix),
                    }
                }
                rules.len() != before
            });

    if let Some(prefix) = orphaned_leading_prefix {
        match key_after_rules(&doc) {
            Some(key) => {
                if let Some(item) = doc.get_mut(&key) {
                    let combined = match item_prefix(item) {
                        Some(existing) => format!("{prefix}{existing}"),
                        None => prefix,
                    };
                    set_item_prefix(item, &combined);
                }
            }
            None => doc.as_table_mut().decor_mut().set_suffix(prefix),
        }
    }

    let mut rebuilt = if dropped_any {
        doc.to_string()
    } else {
        content.to_string()
    };
    if !to_add.is_empty() && !rebuilt.is_empty() && !rebuilt.ends_with('\n') {
        rebuilt.push('\n');
    }
    for block in to_add {
        rebuilt.push_str(block);
    }
    Ok(rebuilt)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &str = "\
# generated header\n\
\n\
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
[[rules]]\n\
name = \"custom-shape\"\n\
# authored comment\n\
assert = { require-section = { heading = \"Goals\" } }\n";

    const SQUAD: &str = "\
[[rules]]\n\
name = \"namespace-unique-squad\"\n\
origin = \"default\"\n\
assert = { require-label = { label = \"squad:*\", min = 0, max = 1 } }\n\n";

    #[test]
    fn test_parse_rule_identities_ignores_assertion_details() {
        let content = "[[rules]]\nname = \"broken\"\norigin = \"custom\"\n\
                       assert = { json-schema = \"missing.json\" }\n";
        assert_eq!(
            parse_rule_identities(content).unwrap(),
            vec![("broken".to_string(), Some("custom".to_string()))]
        );
    }

    #[test]
    fn test_rewrite_header_preserves_rules_and_other_content() {
        let content = "# old\n\n[[rules]]\nname = \"keep\"\n# authored\n\
                       assert = { require-section = { heading = \"H\" } }\n";
        let updated = rewrite_header(content, "# new\n\n").unwrap();
        assert!(updated.starts_with("# new\n\n"));
        assert!(!updated.contains("# old"));
        assert!(updated.contains("name = \"keep\"\n# authored"));
        assert_eq!(rewrite_header(&updated, "# new\n\n").unwrap(), updated);

        let no_rules = "# old\n\n[extra]\nnote = \"keep\"\n";
        assert_eq!(
            rewrite_header(no_rules, "# new\n\n").unwrap(),
            "# new\n\n[extra]\nnote = \"keep\"\n"
        );
        assert_eq!(rewrite_header("\n\n", "# new\n\n").unwrap(), "# new\n\n");
    }

    #[test]
    fn test_splice_add_preserves_original_bytes_and_inserts_separator() {
        assert_eq!(
            splice_default_membership(RULES, &[SQUAD.to_string()], &[]).unwrap(),
            format!("{RULES}{SQUAD}")
        );

        let without_newline = RULES.trim_end_matches('\n');
        let updated =
            splice_default_membership(without_newline, &[SQUAD.to_string()], &[]).unwrap();
        assert!(updated.contains("heading = \"Goals\" } }\n[[rules]]"));
        assert_eq!(parse_rule_identities(&updated).unwrap().len(), 4);
    }

    #[test]
    fn test_splice_drop_only_removes_matching_default_rule() {
        let updated =
            splice_default_membership(RULES, &[], &["namespace-unique-team".to_string()]).unwrap();
        assert!(!updated.contains("name = \"namespace-unique-team\""));
        assert!(updated.contains("# generated header"));
        assert!(updated.contains("name = \"label-format\""));
        assert!(updated.contains("name = \"custom-shape\"\n# authored comment"));

        let custom = "[[rules]]\nname = \"namespace-unique-team\"\n\
                      origin = \"custom\"\nassert = { require-section = { heading = \"H\" } }\n";
        assert_eq!(
            splice_default_membership(custom, &[], &["namespace-unique-team".to_string()]).unwrap(),
            custom
        );
    }

    #[test]
    fn test_splice_add_and_drop_are_one_deterministic_edit() {
        let updated = splice_default_membership(
            RULES,
            &[SQUAD.to_string()],
            &["namespace-unique-team".to_string()],
        )
        .unwrap();
        assert!(!updated.contains("name = \"namespace-unique-team\""));
        assert!(updated.contains("name = \"namespace-unique-squad\""));
        assert!(updated.contains("name = \"custom-shape\"\n# authored comment"));
        assert_eq!(parse_rule_identities(&updated).unwrap().len(), 3);
    }

    #[test]
    fn test_splice_drop_first_or_only_rule_preserves_leading_trivia() {
        let first = "# HEADER\n\n[[rules]]\nname = \"namespace-unique-team\"\n\
                     origin = \"default\"\nassert = { require-section = { heading = \"H\" } }\n\n\
                     [[rules]]\nname = \"custom\"\nassert = { require-section = { heading = \"H\" } }\n";
        let updated =
            splice_default_membership(first, &[], &["namespace-unique-team".to_string()]).unwrap();
        assert!(updated.starts_with("# HEADER\n\n[[rules]]\nname = \"custom\""));

        let only = "# HEADER\n\n[[rules]]\nname = \"namespace-unique-team\"\n\
                    origin = \"default\"\nassert = { require-section = { heading = \"H\" } }\n";
        assert_eq!(
            splice_default_membership(only, &[], &["namespace-unique-team".to_string()]).unwrap(),
            "# HEADER\n\n"
        );
    }

    #[test]
    fn test_splice_drop_preserves_trailing_tables_and_order() {
        let content = "[preamble]\nx = 1\n\n# rules header\n\n[[rules]]\n\
                       name = \"namespace-unique-team\"\norigin = \"default\"\n\
                       assert = { require-section = { heading = \"H\" } }\n\n\
                       [extra]\nnote = \"keep\"\n";
        let updated =
            splice_default_membership(content, &[], &["namespace-unique-team".to_string()])
                .unwrap();
        assert!(updated.starts_with("[preamble]\nx = 1\n"));
        assert!(updated.contains("# rules header"));
        assert!(updated.contains("[extra]\nnote = \"keep\""));
        assert!(parse_rule_identities(&updated).unwrap().is_empty());
    }

    #[test]
    fn test_splice_handles_toml_rule_header_variants_and_embedded_marker() {
        for header in [
            "[[ rules ]]",
            "[[\trules\t]]",
            "[[\"rules\"]]",
            "[['rules']]",
            "[[rules]] # note",
            "  [[rules]]",
        ] {
            let content = format!(
                "[[rules]]\nname = \"default\"\norigin = \"default\"\n\
                 assert = {{ require-section = {{ heading = \"H\" }} }}\n\n\
                 {header}\nname = \"custom\"\ndescription = '''inside\n[[rules]]\n'''\n\
                 assert = {{ require-section = {{ heading = \"H\" }} }}\n"
            );
            let updated = splice_default_membership(&content, &[SQUAD.to_string()], &[]).unwrap();
            assert!(updated.starts_with(&content));
            assert_eq!(parse_rule_identities(&updated).unwrap().len(), 3);
        }

        let non_rule_array = "\
[[rules]]\n\
name = \"default\"\n\
origin = \"default\"\n\
assert = { require-section = { heading = \"H\" } }\n\
\n\
[[ruleset]]\n\
name = \"not-a-rule\"\n";
        let updated = splice_default_membership(non_rule_array, &[SQUAD.to_string()], &[]).unwrap();
        assert!(updated.starts_with(non_rule_array));
        assert!(updated.contains("[[ruleset]]\nname = \"not-a-rule\""));
        assert_eq!(parse_rule_identities(&updated).unwrap().len(), 2);
    }

    #[test]
    fn test_splice_empty_or_missing_drop_is_noop() {
        assert_eq!(splice_default_membership(RULES, &[], &[]).unwrap(), RULES);
        assert_eq!(
            splice_default_membership(RULES, &[], &["missing".to_string()]).unwrap(),
            RULES
        );
    }
}
