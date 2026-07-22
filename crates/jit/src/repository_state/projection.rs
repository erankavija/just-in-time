//! Projection primitives shared by every generic `[projection.<name>]` render.
//!
//! A projection writes a rendered markdown block into a config-selected target in
//! one of two modes:
//!
//! - **separate-file**: a whole file written ATOMICALLY;
//! - **region**: only a delimited region within an existing file is rewritten,
//!   byte-preserving everything OUTSIDE the delimiters.
//!
//! This module holds the mode-independent pieces:
//!
//! - [`render_id_anchor_rows`] — the generic `id-anchor` renderer: heading-less
//!   `- **{self-id}** — {text}` bullets from a kind's addressable items, usable by
//!   ANY item kind with no dedicated code;
//! - [`render_invariants_markdown`] — the built-in `full` render of the invariant
//!   registry (the rule + gate `full` render lives in
//!   [`rules_gates_projection`](crate::repository_state::rules_gates_projection));
//! - [`splice_region`] — the PURE region splice; and
//! - [`require_target`] — resolve a projection's REQUIRED `target`, naming the
//!   projection on omission (no default is applied, REQ-07).
//!
//! Materializing final bytes (branch on
//! [`ProjectionMode`](crate::config::ProjectionMode), splice in region mode)
//! and the atomic write itself are orchestrated by the two-phase
//! [`project_render`](crate::commands) command, which renders and materializes
//! EVERY projection before writing ANY target so a failing render leaves the tree
//! untouched; publication lands through the repository-state transaction.
//!
//! Target path, mode, style, and delimiters come ONLY from
//! [`ProjectionConfig`](crate::config::ProjectionConfig); this module hardcodes no
//! documentation filename. The projection-to-body wiring (which registry or source
//! feeds which projection) lives in
//! [`project_render`](crate::repository_state::projection_render).

use crate::config::{ProjectionConfig, ProjectionMode, ProjectionStyle};
use crate::declarations::invariants::{InvariantKind, InvariantRegistry};
use crate::domain::item::AddressableItem;
use crate::storage::PathReadError;
use anyhow::Result;
use std::collections::BTreeMap;
use thiserror::Error;

/// Errors raised while rendering or writing a generic projection.
///
/// Shared by [`splice_region`] and [`require_target`] here, the body-rendering
/// wiring in [`project_render`](crate::repository_state::projection_render) (kind and source
/// resolution), and the two-phase `jit project render` command that materializes
/// and writes targets. Every variant carries enough context (the
/// offending marker, the target path, the missing projection, the unknown kind, or
/// the underlying I/O error) to point an author at the problem. A missing target,
/// missing/malformed region, source, or kind NEVER silently clobbers a file or
/// writes a partial one: it is a typed error raised BEFORE any write.
#[derive(Debug, Error)]
pub enum ProjectionError {
    /// The configured begin marker was not found in the region-mode target.
    #[error("projection region begin marker '{marker}' not found in target")]
    MissingBeginMarker {
        /// The begin marker that was searched for.
        marker: String,
    },

    /// The configured end marker was not found after the begin marker.
    #[error("projection region end marker '{marker}' not found after begin marker in target")]
    MissingEndMarker {
        /// The end marker that was searched for.
        marker: String,
    },

    /// The end marker appears before the begin marker (malformed region).
    #[error(
        "projection region end marker '{end}' precedes begin marker '{begin}' in target (malformed region)"
    )]
    MarkersOutOfOrder {
        /// The begin marker.
        begin: String,
        /// The end marker.
        end: String,
    },

    /// Region mode requires an existing target file, but none was found.
    #[error(
        "region-mode projection target '{path}' does not exist (region mode cannot create it)"
    )]
    TargetNotFound {
        /// The configured target path.
        path: String,
    },

    /// A projection declared no `target`. The path is required (the engine applies
    /// no default), so this surfaces before any render or write.
    #[error("projection '{projection}' declares no target (a projection must set `target`)")]
    MissingTarget {
        /// The `[projection.<name>]` name that omitted `target`.
        projection: String,
    },

    /// The region-mode target could not be read (invalid path or I/O failure).
    #[error("failed to read projection target '{path}': {source}")]
    Read {
        /// The configured target path.
        path: String,
        /// The underlying typed read error.
        source: PathReadError,
    },

    /// Writing the rendered projection failed (an invalid/escaping path is
    /// rejected as [`PathReadError::InvalidPath`] before any write; an I/O
    /// failure surfaces as [`PathReadError::Other`]).
    #[error("failed to write projection target '{path}': {source}")]
    Write {
        /// The configured target path.
        path: String,
        /// The underlying typed write error.
        source: PathReadError,
    },

    /// A projection declared a `kind` that no `[item_kinds.<name>]` table (or any
    /// kind alias) defines.
    #[error("projection references unknown item kind '{kind}'")]
    UnknownKind {
        /// The unresolved kind name.
        kind: String,
    },

    /// A projection references an item kind that is not project-scoped, so it has
    /// no project-scope source to render from.
    #[error("projection kind '{kind}' is not project-scoped and cannot be projected")]
    NotProjectScoped {
        /// The offending kind name.
        kind: String,
    },

    /// A projected kind's declared source file does not exist, so its rows cannot
    /// be resolved (region-mode rendering never falls back to an empty block).
    #[error("projection source '{path}' for kind '{kind}' does not exist")]
    SourceNotFound {
        /// The configured source path.
        path: String,
        /// The kind whose source is missing.
        kind: String,
    },

    /// The `full` render style is not defined for the projection's declared kind
    /// set (only the built-in invariant and rule+gate registry views exist).
    #[error(
        "the 'full' render style is not available for kind(s) {kinds:?}; \
         use 'id-anchor', or declare the built-in invariant or rule+gate projection"
    )]
    FullStyleUnsupported {
        /// The declared kind names the `full` style could not render.
        kinds: Vec<String>,
    },
}

/// Render `registry` into a deterministic, readable markdown block in `style`.
///
/// Pure: performs no I/O and reads no configuration beyond the `style` argument.
/// Each invariant is listed in authored order. The two styles
/// ([`ProjectionStyle`], REQ-08) differ only in framing:
///
/// - [`ProjectionStyle::Full`] (the default) renders a `## Project invariants`
///   header followed by one `- **{id}** [{kind}]{enforced_by}: {statement}` bullet
///   per invariant, where `enforced_by` is `` (enforced-by: `<rule>`)`` when bound.
///   This output is byte-identical to the original single-style render.
/// - [`ProjectionStyle::IdAnchor`] renders a HEADING-LESS bullet list: one
///   `- **{id}** — {statement}` line per invariant (bold id, space, em-dash, space,
///   statement) with NO kind tag and NO enforced-by, for embedding beneath a
///   hand-authored heading.
///
/// An empty registry renders an explicit "no invariants declared" line so the
/// projected region is never blank (in `Full` style this follows the header; in
/// `IdAnchor` style it is the sole line).
pub fn render_invariants_markdown(registry: &InvariantRegistry, style: ProjectionStyle) -> String {
    match style {
        ProjectionStyle::Full => render_full(registry),
        ProjectionStyle::IdAnchor => render_id_anchor(registry),
    }
}

/// Render the `full` style: header + `- **{id}** [{kind}]{enforced_by}: {statement}`.
///
/// Byte-identical to the original single-style render (REQ-08 keeps `full` the
/// default so existing targets and tests are unchanged).
fn render_full(registry: &InvariantRegistry) -> String {
    let mut out = String::from("## Project invariants\n\n");
    if registry.invariants.is_empty() {
        out.push_str("_No invariants declared._\n");
        return out;
    }
    for inv in &registry.invariants {
        let kind = match inv.kind {
            InvariantKind::Enforced => "enforced",
            InvariantKind::Advisory => "advisory",
        };
        let enforced_by = inv
            .enforced_by
            .as_deref()
            .map(|by| format!(" (enforced-by: `{by}`)"))
            .unwrap_or_default();
        out.push_str(&format!(
            "- **{id}** [{kind}]{enforced_by}: {statement}\n",
            id = inv.id,
            statement = inv.statement,
        ));
    }
    out
}

/// Render the `id-anchor` style: a heading-less `- **{id}** — {statement}` list.
///
/// No `## Project invariants` header, no `[kind]` tag, no enforced-by. The
/// separator is a literal em-dash (`—`). An empty registry still emits the
/// explicit "no invariants declared" line so the projected region is never blank.
fn render_id_anchor(registry: &InvariantRegistry) -> String {
    if registry.invariants.is_empty() {
        return String::from("_No invariants declared._\n");
    }
    registry
        .invariants
        .iter()
        .map(|inv| {
            format!(
                "- **{id}** — {statement}\n",
                id = inv.id,
                statement = inv.statement
            )
        })
        .collect()
}

/// Render `rows` as a heading-less `id-anchor` bullet list — the GENERIC,
/// kind-agnostic projection body any item kind can use with no dedicated code.
///
/// Pure: performs no I/O. Each addressable row becomes one
/// `- **{self-id}** — {display}\n` line, in the order the rows were resolved
/// (registry order for a registry-first kind, document order for a markdown-first
/// kind). `display` is the row `text` with a leading occurrence of its OWN
/// `self-id` and the separator that follows it stripped (see [`id_anchor_display`]),
/// so a row whose source text repeats its self-id — e.g. a charter decision line
/// `D-1: <one-liner>` — renders `- **D-1** — <one-liner>` rather than doubling the
/// id, while a row whose text does not begin with its self-id (e.g. an invariant
/// statement) is rendered verbatim. An empty row set emits an explicit
/// `_No items declared._` line so the projected region is never blank.
pub fn render_id_anchor_rows(rows: &[AddressableItem]) -> String {
    if rows.is_empty() {
        return String::from("_No items declared._\n");
    }
    rows.iter()
        .map(|row| {
            format!(
                "- **{id}** — {display}\n",
                id = row.self_id,
                display = id_anchor_display(&row.self_id, &row.text),
            )
        })
        .collect()
}

/// The display text of an `id-anchor` row: its `text` with a leading `{self_id}`
/// and the separator immediately following it removed, if present.
///
/// A markdown-sourced row's `text` is the whole source list entry, which for a
/// self-labelled item (`D-1: ...`) begins with the self-id. Stripping that prefix
/// plus its trailing separator (`:`, `—`, `-`, or whitespace) yields the bare
/// one-liner. A row whose text does not start with its self-id (e.g. a
/// registry-first invariant, whose `text` is the `statement` field) is returned
/// unchanged.
fn id_anchor_display<'a>(self_id: &str, text: &'a str) -> &'a str {
    match text.strip_prefix(self_id) {
        Some(rest) => {
            rest.trim_start_matches(|c: char| c == ':' || c == '—' || c == '-' || c.is_whitespace())
        }
        None => text,
    }
}

/// Replace the text between `begin` and `end` in `existing` with `rendered`,
/// byte-preserving everything OUTSIDE the delimiters (REQ-01).
///
/// Pure: performs no I/O. The markers themselves are preserved; only the bytes
/// strictly between them are replaced. The rendered block is wrapped in newlines
/// so the markers sit on their own visual lines while the surrounding text (the
/// prefix up to and including `begin`, and the suffix from `end` onward) is
/// returned verbatim. A missing/out-of-order marker is a typed
/// [`ProjectionError`] rather than a silent clobber.
pub fn splice_region(
    existing: &str,
    rendered: &str,
    begin: &str,
    end: &str,
) -> Result<String, ProjectionError> {
    let begin_at = existing
        .find(begin)
        .ok_or_else(|| ProjectionError::MissingBeginMarker {
            marker: begin.to_string(),
        })?;
    let after_begin = begin_at + begin.len();

    // Search for the end marker strictly AFTER the begin marker so a single
    // shared substring cannot be mistaken for both.
    let end_rel = existing[after_begin..].find(end).ok_or_else(|| {
        // Distinguish "end never appears" from "end appears only before begin".
        if existing.contains(end) {
            ProjectionError::MarkersOutOfOrder {
                begin: begin.to_string(),
                end: end.to_string(),
            }
        } else {
            ProjectionError::MissingEndMarker {
                marker: end.to_string(),
            }
        }
    })?;
    let end_at = after_begin + end_rel;

    // Reassemble: [prefix..=begin] + "\n" + rendered + "\n" + [end..suffix].
    // The prefix (through the begin marker) and the suffix (from the end marker
    // on) are sliced byte-exact from `existing`, so content outside the region is
    // byte-preserved.
    let prefix = &existing[..after_begin];
    let suffix = &existing[end_at..];
    Ok(format!(
        "{prefix}\n{rendered}\n{suffix}",
        rendered = rendered.trim_end_matches('\n')
    ))
}

/// Compose one projection's rendered `body` into `pending`, threading region
/// splices through progressively-updated target content.
///
/// `pending` maps each repo-relative target path to its in-progress content. In
/// region mode the splice base is the target's PENDING content when another
/// projection already rewrote it this pass, otherwise the bytes `read_base`
/// returns (a `None` there is a typed [`ProjectionError::TargetNotFound`], since
/// region mode cannot create a target); separate-file mode makes `body` the whole
/// target. The composed content is stored back in `pending`, so several
/// projections sharing ONE target each observe the prior one's change rather than
/// overwriting it (last-writer-wins). Shared by the two-phase `jit project render`
/// command and profile planning's projection re-render so both compose shared
/// targets identically.
pub fn compose_projection(
    pending: &mut BTreeMap<String, String>,
    target: &str,
    mode: ProjectionMode,
    body: &str,
    begin: &str,
    end: &str,
    read_base: impl FnOnce(&str) -> Result<Option<String>>,
) -> Result<()> {
    let content = match mode {
        ProjectionMode::SeparateFile => body.to_string(),
        ProjectionMode::Region => {
            let base = match pending.get(target) {
                Some(current) => current.clone(),
                None => read_base(target)?.ok_or_else(|| ProjectionError::TargetNotFound {
                    path: target.to_string(),
                })?,
            };
            splice_region(&base, body, begin, end)?
        }
    };
    pending.insert(target.to_string(), content);
    Ok(())
}

/// Resolve a projection's required `target`, naming the projection on omission.
///
/// The engine applies no default target (REQ-07): a `[projection.<name>]` table
/// with no `target` is a typed [`ProjectionError::MissingTarget`] raised before any
/// render or write, wherever a projection's target is needed (the `jit project
/// render` command, `jit validate`'s freshness check, and profile planning).
pub fn require_target(
    projection: &ProjectionConfig,
    name: &str,
) -> Result<String, ProjectionError> {
    projection
        .target
        .clone()
        .ok_or_else(|| ProjectionError::MissingTarget {
            projection: name.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectionStyle;

    fn registry_with_two() -> InvariantRegistry {
        InvariantRegistry::from_toml_str(
            r#"
[[invariants]]
id = "sample-invariant"
statement = "Every dependency edge stays acyclic."
kind = "enforced"
enforced-by = "dag-no-cycles"

[[invariants]]
id = "second-invariant"
statement = "Issues prefer functional style."
kind = "advisory"
"#,
        )
        .unwrap()
    }

    #[test]
    fn test_render_lists_each_invariant_deterministically() {
        let md = render_invariants_markdown(&registry_with_two(), ProjectionStyle::Full);
        // Authored order is preserved: sample-invariant before second-invariant.
        let p1 = md.find("sample-invariant").unwrap();
        let p2 = md.find("second-invariant").unwrap();
        assert!(p1 < p2);
        assert!(md.contains("[enforced]"));
        assert!(md.contains("[advisory]"));
        assert!(md.contains("enforced-by: `dag-no-cycles`"));
        assert!(md.contains("Issues prefer functional style."));
        // Deterministic: same input renders identical output.
        assert_eq!(
            md,
            render_invariants_markdown(&registry_with_two(), ProjectionStyle::Full)
        );
    }

    #[test]
    fn test_render_empty_registry_has_explicit_line() {
        let md = render_invariants_markdown(&InvariantRegistry::empty(), ProjectionStyle::Full);
        assert!(md.contains("No invariants declared"));
    }

    #[test]
    fn test_render_default_style_is_full() {
        // REQ-08: an absent `style` field resolves to Full, so the default render
        // is byte-identical to the explicit full render.
        let reg = registry_with_two();
        let via_default = render_invariants_markdown(&reg, ProjectionStyle::default());
        let explicit_full = render_invariants_markdown(&reg, ProjectionStyle::Full);
        assert_eq!(via_default, explicit_full);
        assert!(via_default.starts_with("## Project invariants\n\n"));
    }

    #[test]
    fn test_render_id_anchor_is_heading_less_with_no_kind_or_enforced_by() {
        // REQ-08: id-anchor renders `- **{id}** — {statement}` bullets with no
        // header, no [kind] tag, and no enforced-by, in authored order.
        let md = render_invariants_markdown(&registry_with_two(), ProjectionStyle::IdAnchor);
        assert_eq!(
            md,
            "- **sample-invariant** — Every dependency edge stays acyclic.\n\
             - **second-invariant** — Issues prefer functional style.\n"
        );
        // No header, no kind tag, no enforced-by leaks into the id-anchor render.
        assert!(!md.contains("## Project invariants"));
        assert!(!md.contains("[enforced]"));
        assert!(!md.contains("[advisory]"));
        assert!(!md.contains("enforced-by"));
        // The separator is a literal em-dash with surrounding spaces.
        assert!(md.contains("** — "));
    }

    #[test]
    fn test_render_id_anchor_empty_registry_has_explicit_line() {
        let md = render_invariants_markdown(&InvariantRegistry::empty(), ProjectionStyle::IdAnchor);
        assert_eq!(md, "_No invariants declared._\n");
        assert!(!md.contains("## Project invariants"));
    }

    #[test]
    fn test_splice_region_byte_preserves_outside_and_replaces_inside() {
        // The surrounding bytes (prefix and suffix) must be byte-identical after
        // the splice; only the region between the markers changes (REQ-01).
        let begin = "<!-- jit:invariants:begin -->";
        let end = "<!-- jit:invariants:end -->";
        let prefix = "# My Doc\n\nSome intro prose.\n\n";
        let suffix = "\n\n## After\n\nTrailing content with trailing newline.\n";
        let existing = format!("{prefix}{begin}\nOLD INNER\n{end}{suffix}");

        let out = splice_region(&existing, "NEW INNER", begin, end).unwrap();

        // Outside-the-region bytes are preserved EXACTLY.
        assert!(out.starts_with(&format!("{prefix}{begin}")));
        assert!(out.ends_with(&format!("{end}{suffix}")));
        // Inside the region was correctly replaced.
        assert!(out.contains("NEW INNER"));
        assert!(!out.contains("OLD INNER"));
        // Markers themselves survive.
        assert!(out.contains(begin));
        assert!(out.contains(end));

        // Strong byte-preservation: reconstruct from the known prefix/suffix and
        // compare the surrounding bytes literally.
        let inner_start = out.find(begin).unwrap();
        let inner_end = out.find(end).unwrap() + end.len();
        assert_eq!(
            &out[..inner_start + begin.len()],
            format!("{prefix}{begin}")
        );
        assert_eq!(&out[inner_end..], suffix);
    }

    #[test]
    fn test_splice_region_missing_begin_is_typed_error() {
        let err = splice_region("no markers", "X", "<!--b-->", "<!--e-->").unwrap_err();
        assert!(matches!(err, ProjectionError::MissingBeginMarker { .. }));
    }

    #[test]
    fn test_splice_region_missing_end_is_typed_error() {
        let err = splice_region("pre <!--b--> post", "X", "<!--b-->", "<!--e-->").unwrap_err();
        assert!(matches!(err, ProjectionError::MissingEndMarker { .. }));
    }

    #[test]
    fn test_splice_region_out_of_order_is_typed_error() {
        // End appears, but only BEFORE begin.
        let err = splice_region("<!--e--> ... <!--b-->", "X", "<!--b-->", "<!--e-->").unwrap_err();
        assert!(matches!(err, ProjectionError::MarkersOutOfOrder { .. }));
    }

    /// Build an addressable row with the given self-id and text (project scope,
    /// invariant kind), enough to exercise the id-anchor row renderer.
    fn row(self_id: &str, text: &str) -> AddressableItem {
        AddressableItem {
            kind: "invariant".to_string(),
            qualified_id: format!("@/invariant/{self_id}"),
            self_id: self_id.to_string(),
            scope: "@".to_string(),
            text: text.to_string(),
            links: Vec::new(),
        }
    }

    #[test]
    fn test_render_id_anchor_rows_keeps_text_without_self_id_prefix() {
        // A row whose text does NOT begin with its self-id (a registry-first
        // invariant, whose text is the statement) is rendered verbatim.
        let rows = [
            row("label-format", "Every label is namespace:value."),
            row("dag-acyclic", "The dependency graph stays acyclic."),
        ];
        assert_eq!(
            render_id_anchor_rows(&rows),
            "- **label-format** — Every label is namespace:value.\n\
             - **dag-acyclic** — The dependency graph stays acyclic.\n"
        );
    }

    #[test]
    fn test_render_id_anchor_rows_strips_repeated_self_id() {
        // A markdown-sourced row whose text repeats its self-id (a charter decision
        // line `D-1: ...`) renders `- **D-1** — <one-liner>`, not a doubled id.
        let rows = [
            row("D-1", "D-1: Repository-local git-versioned JSON storage"),
            row(
                "D-6",
                "D-6: Each item kind declares its own source of truth",
            ),
        ];
        assert_eq!(
            render_id_anchor_rows(&rows),
            "- **D-1** — Repository-local git-versioned JSON storage\n\
             - **D-6** — Each item kind declares its own source of truth\n"
        );
    }

    #[test]
    fn test_render_id_anchor_rows_empty_is_explicit_line() {
        assert_eq!(render_id_anchor_rows(&[]), "_No items declared._\n");
    }

    /// Parity oracle (REQ-08): the generic id-anchor row renderer reproduces the
    /// typed invariant id-anchor render, computed LIVE from the same registry —
    /// two independent code paths, not a committed-doc mirror. This is what keeps
    /// the migrated `invariants` projection byte-identical to its prior output.
    #[test]
    fn test_render_id_anchor_rows_matches_typed_invariant_render() {
        let reg = registry_with_two();
        let rows: Vec<AddressableItem> = reg
            .invariants
            .iter()
            .map(|inv| row(&inv.id, &inv.statement))
            .collect();
        assert_eq!(
            render_id_anchor_rows(&rows),
            render_invariants_markdown(&reg, ProjectionStyle::IdAnchor)
        );
    }
}
