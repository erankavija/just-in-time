//! Pure span-level `rules.toml` document edits (byte in, byte out).
//!
//! The default-rule family is a projection of the namespace registry, but the
//! file also carries hand-authored content — custom rules, comments, the header,
//! block order, and the editable policy fields (`severity`/`enforce`/selector) of
//! generated rules. These primitives edit ONLY the generated spans and preserve
//! every other byte: [`rewrite_header`] republishes the leading generated comment
//! region, and [`splice_default_membership`] appends newly generated
//! `namespace-unique-*` blocks and drops obsolete `origin = "default"` ones. Both
//! are pure `&str -> String` transforms so the same edit runs in the mutation
//! derive pipeline and behind the storage read/write boundary without a second
//! implementation.

use crate::declarations::rules::DEFAULT_ORIGIN;
use anyhow::{Context, Result};

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
