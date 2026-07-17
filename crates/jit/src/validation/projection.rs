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
//!   [`rules_gates_projection`](crate::validation::rules_gates_projection));
//! - [`splice_region`] — the PURE region splice; and
//! - [`write_projection`] — the sole I/O orchestrator: it branches on
//!   [`ProjectionMode`], splices in region mode, and writes atomically through the
//!   storage boundary
//!   ([`IssueStore::write_repo_file`](crate::storage::IssueStore::write_repo_file),
//!   itself over [`write_file_atomic`](crate::validation::serialize::write_file_atomic)).
//!
//! Target path, mode, style, and delimiters come ONLY from
//! [`ProjectionConfig`](crate::config::ProjectionConfig); this module hardcodes no
//! documentation filename. The projection-to-body wiring (which registry or source
//! feeds which projection) lives in
//! [`project_render`](crate::validation::project_render).

use crate::config::{ProjectionMode, ProjectionStyle};
use crate::domain::item::AddressableItem;
use crate::storage::{IssueStore, PathReadError};
use crate::validation::invariants::{InvariantKind, InvariantRegistry};
use thiserror::Error;

/// Errors raised while rendering or writing a generic projection.
///
/// Shared by [`write_projection`] (the region splice + atomic write) and the
/// body-rendering wiring in
/// [`project_render`](crate::validation::project_render) (kind resolution + source
/// reads). Every variant carries enough context (the offending marker, the target
/// path, the unknown kind, or the underlying I/O error) to point an author at the
/// problem. A missing/malformed region, source, or kind NEVER silently clobbers a
/// file or writes a partial one: it is a typed error raised BEFORE any write.
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

/// Write an already-rendered projection `body` into `target` under `mode`.
///
/// The sole I/O orchestrator, generic over every projection kind and style: it
/// takes the pre-rendered markdown `body` and, per `mode`,
///
/// - **separate-file**: atomic-writes the whole file to `target`;
/// - **region**: reads the existing `target` through the storage boundary, splices
///   `body` between `region_begin` / `region_end` (byte-preserving everything
///   outside via [`splice_region`]), then atomic-writes the result.
///
/// ALL persistence goes through [`write_repo_file`], which path-validates the
/// config-driven `target` (rejecting an absolute or `..`-escaping path) BEFORE any
/// write and writes atomically through the shared
/// [`write_file_atomic`](crate::validation::serialize::write_file_atomic). A
/// missing region target or missing/malformed delimiters is a typed
/// [`ProjectionError`] raised before writing — the file is never silently
/// clobbered or left partially written. Returns the repo-relative path written.
///
/// [`read_repo_file`]: crate::storage::IssueStore::read_repo_file
/// [`write_repo_file`]: crate::storage::IssueStore::write_repo_file
pub fn write_projection<S: IssueStore>(
    store: &S,
    mode: ProjectionMode,
    target: &str,
    region_begin: &str,
    region_end: &str,
    body: &str,
) -> Result<String, ProjectionError> {
    let content = match mode {
        ProjectionMode::SeparateFile => body.to_string(),
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
            splice_region(&existing, body, region_begin, region_end)?
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
    use crate::config::{ProjectionMode, ProjectionStyle};
    use crate::storage::JsonFileStorage;

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

    /// A body written in separate-file mode lands atomically at the target.
    #[test]
    fn test_write_projection_separate_file_writes_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        let body = render_invariants_markdown(&registry_with_two(), ProjectionStyle::Full);
        // The write goes through storage, which creates intermediate dirs (the
        // repo root is the parent of `.jit`).
        let written = write_projection(
            &store,
            ProjectionMode::SeparateFile,
            "docs/invariants.md",
            "",
            "",
            &body,
        )
        .unwrap();
        assert_eq!(written, "docs/invariants.md");

        let on_disk = std::fs::read_to_string(dir.path().join("docs/invariants.md")).unwrap();
        assert!(on_disk.contains("sample-invariant"));
        assert!(on_disk.contains("second-invariant"));
        // No leftover temp file (atomic temp+rename leaves only the target).
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("docs"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no .tmp temp file should remain");
    }

    #[test]
    fn test_write_projection_region_byte_preserves_surrounding_file() {
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        let begin = "<!-- jit:invariants:begin -->";
        let end = "<!-- jit:invariants:end -->";
        let prefix = "# Hand-written doc\n\nIntro the user wrote.\n\n";
        let suffix = "\n\n## Footer\n\nMore hand-written prose.\n";
        let original = format!("{prefix}{begin}\nstale\n{end}{suffix}");
        std::fs::write(dir.path().join("GUIDE.md"), &original).unwrap();

        let body = render_invariants_markdown(&registry_with_two(), ProjectionStyle::Full);
        write_projection(
            &store,
            ProjectionMode::Region,
            "GUIDE.md",
            begin,
            end,
            &body,
        )
        .unwrap();

        let updated = std::fs::read_to_string(dir.path().join("GUIDE.md")).unwrap();
        // Surrounding bytes preserved exactly.
        assert!(updated.starts_with(&format!("{prefix}{begin}")));
        assert!(updated.ends_with(&format!("{end}{suffix}")));
        // Region replaced.
        assert!(updated.contains("sample-invariant"));
        assert!(!updated.contains("stale"));
    }

    #[test]
    fn test_write_projection_region_missing_target_is_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        let err = write_projection(
            &store,
            ProjectionMode::Region,
            "MISSING.md",
            "<!--b-->",
            "<!--e-->",
            "body",
        )
        .unwrap_err();
        assert!(matches!(err, ProjectionError::TargetNotFound { .. }));
    }

    #[test]
    fn test_write_projection_separate_file_rejects_escaping_target() {
        // A separate-file target that escapes the repo (absolute or `..`) is
        // rejected by the storage path validator BEFORE any write — nothing is
        // written outside the repo.
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        for bad in ["../escape.md", "/tmp/jit-escape.md"] {
            let err = write_projection(&store, ProjectionMode::SeparateFile, bad, "", "", "body")
                .unwrap_err();
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
        // Nothing leaked outside the repo.
        assert!(!dir.path().join("../escape.md").exists());
    }

    /// REQ-06: with region markers in place and `mode=region, target=AGENTS.md`,
    /// the write replaces ONLY the marked region.
    #[test]
    fn test_write_projection_region_into_agents_md_scenario() {
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        // Simulate AGENTS.md: hand-authored content wrapping a jit-managed region.
        let begin = "<!-- jit:charter:begin -->";
        let end = "<!-- jit:charter:end -->";
        let preamble = "# AGENTS.md\n\n### Charter Decisions\n\n";
        let postamble = "\n\n## Commit Conventions\n\nRun cargo fmt.\n";
        let original = format!("{preamble}{begin}\nstale hand-authored prose\n{end}{postamble}");
        std::fs::write(dir.path().join("AGENTS.md"), &original).unwrap();

        let body = render_id_anchor_rows(&[row("D-1", "D-1: JSON-in-git storage")]);
        let written = write_projection(
            &store,
            ProjectionMode::Region,
            "AGENTS.md",
            begin,
            end,
            &body,
        )
        .unwrap();
        assert_eq!(written, "AGENTS.md");

        let updated = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        // Bytes OUTSIDE the region are byte-identical (REQ-01).
        assert!(updated.starts_with(&format!("{preamble}{begin}")));
        assert!(updated.ends_with(&format!("{end}{postamble}")));
        // Stale hand-authored prose was replaced with the projected row.
        assert!(!updated.contains("stale hand-authored prose"));
        assert!(updated.contains("- **D-1** — JSON-in-git storage"));
        // Markers themselves survive.
        assert!(updated.contains(begin));
        assert!(updated.contains(end));
    }

    /// REQ-07: malformed/missing markers raise a typed `ProjectionError` WITHOUT
    /// clobbering the file (the file must remain byte-identical after the error).
    #[test]
    fn test_write_projection_region_malformed_markers_do_not_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        let original = "# AGENTS.md\n\nNo markers here.\n\n## Commit Conventions\n";
        std::fs::write(dir.path().join("AGENTS.md"), original).unwrap();

        // Missing begin marker is a typed error.
        let err = write_projection(
            &store,
            ProjectionMode::Region,
            "AGENTS.md",
            "<!-- jit:x:begin -->",
            "<!-- jit:x:end -->",
            "body",
        )
        .unwrap_err();
        assert!(
            matches!(err, ProjectionError::MissingBeginMarker { .. }),
            "expected MissingBeginMarker, got {err:?}"
        );

        // File is byte-identical — no clobber.
        let after = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert_eq!(
            after, original,
            "file must not be modified when markers are missing"
        );
    }
}
