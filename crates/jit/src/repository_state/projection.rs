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

use crate::config::{ProjectionConfig, ProjectionStyle};
use crate::declarations::invariants::{InvariantKind, InvariantRegistry};
use crate::domain::item::AddressableItem;
use thiserror::Error;

/// Errors raised while rendering or writing a generic projection.
///
/// Shared by [`require_target`] here, the body-rendering wiring in
/// [`project_render`](crate::repository_state::projection_render), and the
/// transaction-backed project-render command. Managed-region structure is
/// validated exclusively by
/// [`managed_document`](crate::repository_state::managed_document).
#[derive(Debug, Error)]
pub enum ProjectionError {
    /// A projection declared no `target`. The path is required (the engine applies
    /// no default), so this surfaces before any render or write.
    #[error("projection '{projection}' declares no target (a projection must set `target`)")]
    MissingTarget {
        /// The `[projection.<name>]` name that omitted `target`.
        projection: String,
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
