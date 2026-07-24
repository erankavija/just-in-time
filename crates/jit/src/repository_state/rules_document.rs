//! Pure span-level `rules.toml` document edits (byte in, byte out).
//!
//! The default-rule family is a projection of the namespace registry, but the
//! file also carries hand-authored content — custom rules, comments, the header,
//! block order, and the editable policy fields (`severity`/`enforce`/selector) of
//! generated rules. These primitives edit ONLY the generated spans and preserve
//! every other byte: [`rewrite_default_assertions`] replaces assertion values in
//! proven `origin = "default"` blocks, and [`splice_default_membership`] appends
//! newly generated `namespace-unique-*` blocks and drops obsolete default ones.
//! Both are pure `&str -> String` transforms consumed directly by the mutation
//! derive pipeline.

use crate::declarations::rules::DEFAULT_ORIGIN;
use serde::Deserialize;

/// A typed failure raised while parsing or span-editing a `rules.toml` document.
///
/// Every variant carries its concrete parse source (or none, for a structural
/// absence); rendering lives in this type's `Display` impl rather than at the call
/// site. Composed into [`ProducerError::RulesDocument`](super::ProducerError).
#[derive(Debug, thiserror::Error)]
pub enum RulesDocumentError {
    /// `rules.toml` could not be parsed for `(name, origin)` rule identities.
    #[error("parsing rule identities from rules.toml")]
    ParseIdentities(#[source] toml::de::Error),
    /// The authored `rules.toml` could not be parsed as a TOML document.
    #[error("parsing rules.toml as TOML")]
    ParseDocument(#[source] toml_edit::TomlError),
    /// The derived default `rules.toml` could not be parsed as a TOML document.
    #[error("parsing derived default rules.toml")]
    ParseDerived(#[source] toml_edit::TomlError),
    /// The derived default `rules.toml` carried no `rules` array.
    #[error("derived default rules.toml has no rules array")]
    DerivedMissingRulesArray,
    /// A proven default-origin rule carried no assertion value.
    #[error("default-origin rule has no assertion value")]
    DefaultRuleMissingAssertion,
    /// A derived default rule carried no assertion value.
    #[error("derived default rule has no assertion value")]
    DerivedRuleMissingAssertion,
}

type Result<T> = std::result::Result<T, RulesDocumentError>;

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
        toml::from_str(content).map_err(RulesDocumentError::ParseIdentities)?;
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

/// Replace only the generated `assert` value of each proven default-origin rule.
///
/// `expected` is the canonical default-only rules document produced by the one
/// rules serializer. The existing value's decoration is retained, preserving
/// whitespace and comments around the generated value; every other source byte
/// (including the file header, block order, and editable policy fields) remains
/// under authored ownership.
pub(super) fn rewrite_default_assertions(content: &str, expected: &str) -> Result<String> {
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(RulesDocumentError::ParseDocument)?;
    let expected = expected
        .parse::<toml_edit::DocumentMut>()
        .map_err(RulesDocumentError::ParseDerived)?;
    let expected_rules = expected
        .get(RULES_ARRAY_KEY)
        .and_then(toml_edit::Item::as_array_of_tables)
        .ok_or(RulesDocumentError::DerivedMissingRulesArray)?;
    let Some(rules) = doc
        .get_mut(RULES_ARRAY_KEY)
        .and_then(toml_edit::Item::as_array_of_tables_mut)
    else {
        return Ok(content.to_string());
    };
    for table in rules.iter_mut() {
        if table.get("origin").and_then(toml_edit::Item::as_str) != Some(DEFAULT_ORIGIN) {
            continue;
        }
        let Some(name) = table.get("name").and_then(toml_edit::Item::as_str) else {
            continue;
        };
        let Some(mut replacement) = expected_rules
            .iter()
            .find(|expected| expected.get("name").and_then(toml_edit::Item::as_str) == Some(name))
            .and_then(|expected| expected.get("assert"))
            .cloned()
        else {
            continue;
        };
        let current = table
            .get("assert")
            .and_then(toml_edit::Item::as_value)
            .ok_or(RulesDocumentError::DefaultRuleMissingAssertion)?;
        let replacement_value = replacement
            .as_value_mut()
            .ok_or(RulesDocumentError::DerivedRuleMissingAssertion)?;
        *replacement_value.decor_mut() = current.decor().clone();
        table["assert"] = replacement;
    }
    Ok(doc.to_string())
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
        .map_err(RulesDocumentError::ParseDocument)?;

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
# authored header\n\
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
        assert!(updated.contains("# authored header"));
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
