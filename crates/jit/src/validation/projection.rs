//! Project the loaded invariant registry into a CONFIGURABLE documentation target.
//!
//! The registry ([`InvariantRegistry`]) renders to a readable markdown block that
//! is written into one of two config-selected targets (decision D3, REQ-06):
//!
//! - **separate-file** (the shipped DEFAULT): a jit-owned file written ATOMICALLY,
//!   so the default behavior never touches existing docs;
//! - **region**: only a delimited region within an existing file is rewritten,
//!   byte-preserving everything OUTSIDE the delimiters.
//!
//! The target path, mode, and region delimiters come ONLY from
//! [`InvariantProjectionConfig`]; this module hardcodes NO documentation filename
//! (REQ-04). Writes go through the shared atomic writer
//! [`write_file_atomic`](crate::validation::serialize::write_file_atomic) (REQ-02,
//! REQ-05); region mode reads the existing target through
//! [`IssueStore::read_repo_file`](crate::storage::IssueStore::read_repo_file).
//!
//! Rendering ([`render_invariants_markdown`]) and the region splice
//! ([`splice_region`]) are PURE and unit-testable; the orchestrator
//! [`project_invariants`] is the only function that performs I/O.

use crate::config::{InvariantProjectionConfig, ProjectionMode};
use crate::storage::{IssueStore, PathReadError};
use crate::validation::invariants::{InvariantKind, InvariantRegistry};
use thiserror::Error;

/// Errors raised while projecting the invariant registry into its doc target.
///
/// Every variant carries enough context (the offending marker, the target path,
/// or the underlying I/O error) to point an author at the problem. A missing or
/// malformed region NEVER silently clobbers the file: it is a typed error.
///
/// # Examples
///
/// ```
/// use jit::validation::projection::{splice_region, ProjectionError};
///
/// // A target whose begin marker is absent is a typed error, not a clobber.
/// let err = splice_region("no markers here", "X", "<!--b-->", "<!--e-->").unwrap_err();
/// assert!(matches!(err, ProjectionError::MissingBeginMarker { .. }));
/// ```
#[derive(Debug, Error)]
pub enum ProjectionError {
    /// The configured begin marker was not found in the region-mode target.
    #[error("invariant region begin marker '{marker}' not found in target")]
    MissingBeginMarker {
        /// The begin marker that was searched for.
        marker: String,
    },

    /// The configured end marker was not found after the begin marker.
    #[error("invariant region end marker '{marker}' not found after begin marker in target")]
    MissingEndMarker {
        /// The end marker that was searched for.
        marker: String,
    },

    /// The end marker appears before the begin marker (malformed region).
    #[error(
        "invariant region end marker '{end}' precedes begin marker '{begin}' in target (malformed region)"
    )]
    MarkersOutOfOrder {
        /// The begin marker.
        begin: String,
        /// The end marker.
        end: String,
    },

    /// Region mode requires an existing target file, but none was found.
    #[error("region-mode invariant target '{path}' does not exist (region mode cannot create it)")]
    TargetNotFound {
        /// The configured target path.
        path: String,
    },

    /// The region-mode target could not be read (invalid path or I/O failure).
    #[error("failed to read invariant projection target '{path}': {source}")]
    Read {
        /// The configured target path.
        path: String,
        /// The underlying typed read error.
        source: PathReadError,
    },

    /// Writing the rendered projection failed (an invalid/escaping path is
    /// rejected as [`PathReadError::InvalidPath`] before any write; an I/O
    /// failure surfaces as [`PathReadError::Other`]).
    #[error("failed to write invariant projection target '{path}': {source}")]
    Write {
        /// The configured target path.
        path: String,
        /// The underlying typed write error.
        source: PathReadError,
    },
}

/// Render `registry` into a deterministic, readable markdown block.
///
/// Pure: performs no I/O and reads no configuration. Each invariant is listed in
/// authored order with its id, statement, kind, and (when bound) the rule/gate
/// that enforces it. An empty registry renders a header plus an explicit "no
/// invariants declared" line so the projected region is never blank.
///
/// # Examples
///
/// ```
/// use jit::validation::invariants::InvariantRegistry;
/// use jit::validation::projection::render_invariants_markdown;
///
/// let reg = InvariantRegistry::from_toml_str(
///     "[[invariants]]\nid = \"INV-01\"\nstatement = \"Acyclic.\"\nkind = \"enforced\"\nenforced-by = \"dag-no-cycles\"\n",
/// )
/// .unwrap();
/// let md = render_invariants_markdown(&reg);
/// assert!(md.contains("INV-01"));
/// assert!(md.contains("Acyclic."));
/// assert!(md.contains("enforced"));
/// assert!(md.contains("dag-no-cycles"));
/// ```
pub fn render_invariants_markdown(registry: &InvariantRegistry) -> String {
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

/// Replace the text between `begin` and `end` in `existing` with `rendered`,
/// byte-preserving everything OUTSIDE the delimiters (REQ-01).
///
/// Pure: performs no I/O. The markers themselves are preserved; only the bytes
/// strictly between them are replaced. The rendered block is wrapped in newlines
/// so the markers sit on their own visual lines while the surrounding text (the
/// prefix up to and including `begin`, and the suffix from `end` onward) is
/// returned verbatim. A missing/out-of-order marker is a typed
/// [`ProjectionError`] rather than a silent clobber.
///
/// # Examples
///
/// ```
/// use jit::validation::projection::splice_region;
///
/// let existing = "intro\n<!--b-->\nOLD\n<!--e-->\noutro\n";
/// let out = splice_region(existing, "NEW", "<!--b-->", "<!--e-->").unwrap();
/// assert!(out.starts_with("intro\n<!--b-->"));
/// assert!(out.ends_with("<!--e-->\noutro\n"));
/// assert!(out.contains("NEW"));
/// assert!(!out.contains("OLD"));
/// ```
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

/// Project `registry` into the documentation target described by `config`.
///
/// The orchestrator (the only function here that performs I/O) reads the target
/// path, mode, and delimiters ONLY from `config` — this module contains no
/// documentation-filename literal (REQ-04). ALL persistence goes through the
/// storage boundary: region mode reads the existing target via
/// [`read_repo_file`] and both modes write via [`write_repo_file`], which
/// path-validates the config-driven target (rejecting absolute/`..`-escaping
/// paths) and writes atomically through the shared
/// [`write_file_atomic`](crate::validation::serialize::write_file_atomic) (REQ-05).
/// No direct filesystem access happens here.
///
/// - **separate-file**: render → atomic write of the whole file.
/// - **region**: read the existing target, splice the rendered block between the
///   configured delimiters (byte-preserving everything outside), atomic-write the
///   result. A missing target or missing/malformed delimiters is a typed
///   [`ProjectionError`] — the file is never silently clobbered.
///
/// Returns the repo-relative path that was written.
///
/// [`read_repo_file`]: crate::storage::IssueStore::read_repo_file
/// [`write_repo_file`]: crate::storage::IssueStore::write_repo_file
///
/// # Examples
///
/// ```no_run
/// use jit::config::InvariantProjectionConfig;
/// use jit::storage::JsonFileStorage;
/// use jit::validation::invariants::InvariantRegistry;
/// use jit::validation::projection::project_invariants;
///
/// let store = JsonFileStorage::new(".jit");
/// let cfg = InvariantProjectionConfig::default();
/// let reg = InvariantRegistry::empty();
/// // Writes the rendered registry to the configured (default jit-owned) target.
/// let written = project_invariants(&store, &cfg, &reg).unwrap();
/// println!("projected invariants to {written}");
/// ```
pub fn project_invariants<S: IssueStore>(
    store: &S,
    config: &InvariantProjectionConfig,
    registry: &InvariantRegistry,
) -> Result<String, ProjectionError> {
    let target = config.target();
    let rendered = render_invariants_markdown(registry);

    let content = match config.mode() {
        ProjectionMode::SeparateFile => rendered,
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
            splice_region(
                &existing,
                &rendered,
                config.region_begin(),
                config.region_end(),
            )?
        }
    };

    // Persist through the storage boundary: it path-validates the config-driven
    // target (rejecting an absolute or `..`-escaping path) BEFORE writing, and
    // writes atomically via the shared writer.
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
    use crate::config::ProjectionMode;
    use crate::storage::JsonFileStorage;

    fn registry_with_two() -> InvariantRegistry {
        InvariantRegistry::from_toml_str(
            r#"
[[invariants]]
id = "INV-01"
statement = "Every dependency edge stays acyclic."
kind = "enforced"
enforced-by = "dag-no-cycles"

[[invariants]]
id = "INV-02"
statement = "Issues prefer functional style."
kind = "advisory"
"#,
        )
        .unwrap()
    }

    #[test]
    fn test_render_lists_each_invariant_deterministically() {
        let md = render_invariants_markdown(&registry_with_two());
        // Authored order is preserved: INV-01 before INV-02.
        let p1 = md.find("INV-01").unwrap();
        let p2 = md.find("INV-02").unwrap();
        assert!(p1 < p2);
        assert!(md.contains("[enforced]"));
        assert!(md.contains("[advisory]"));
        assert!(md.contains("enforced-by: `dag-no-cycles`"));
        assert!(md.contains("Issues prefer functional style."));
        // Deterministic: same input renders identical output.
        assert_eq!(md, render_invariants_markdown(&registry_with_two()));
    }

    #[test]
    fn test_render_empty_registry_has_explicit_line() {
        let md = render_invariants_markdown(&InvariantRegistry::empty());
        assert!(md.contains("No invariants declared"));
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

    #[test]
    fn test_project_separate_file_writes_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        let cfg = InvariantProjectionConfig {
            mode: Some(ProjectionMode::SeparateFile),
            target: Some("docs/invariants.md".to_string()),
            ..Default::default()
        };
        // The write goes through storage, which creates intermediate dirs (the
        // repo root is the parent of `.jit`).
        let written = project_invariants(&store, &cfg, &registry_with_two()).unwrap();
        assert_eq!(written, "docs/invariants.md");

        let on_disk = std::fs::read_to_string(dir.path().join("docs/invariants.md")).unwrap();
        assert!(on_disk.contains("INV-01"));
        assert!(on_disk.contains("INV-02"));
        // No leftover temp file (atomic temp+rename leaves only the target).
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("docs"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no .tmp temp file should remain");
    }

    #[test]
    fn test_project_region_byte_preserves_surrounding_file() {
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

        let cfg = InvariantProjectionConfig {
            mode: Some(ProjectionMode::Region),
            target: Some("GUIDE.md".to_string()),
            region_begin: Some(begin.to_string()),
            region_end: Some(end.to_string()),
        };
        project_invariants(&store, &cfg, &registry_with_two()).unwrap();

        let updated = std::fs::read_to_string(dir.path().join("GUIDE.md")).unwrap();
        // Surrounding bytes preserved exactly.
        assert!(updated.starts_with(&format!("{prefix}{begin}")));
        assert!(updated.ends_with(&format!("{end}{suffix}")));
        // Region replaced.
        assert!(updated.contains("INV-01"));
        assert!(!updated.contains("stale"));
    }

    #[test]
    fn test_project_region_missing_target_is_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        let cfg = InvariantProjectionConfig {
            mode: Some(ProjectionMode::Region),
            target: Some("MISSING.md".to_string()),
            ..Default::default()
        };
        let err = project_invariants(&store, &cfg, &registry_with_two()).unwrap_err();
        assert!(matches!(err, ProjectionError::TargetNotFound { .. }));
    }

    #[test]
    fn test_project_separate_file_rejects_escaping_target() {
        // A separate-file target that escapes the repo (absolute or `..`) is
        // rejected by the storage path validator BEFORE any write — nothing is
        // written outside the repo.
        let dir = tempfile::tempdir().unwrap();
        let jit_root = dir.path().join(".jit");
        std::fs::create_dir_all(&jit_root).unwrap();
        let store = JsonFileStorage::new(&jit_root);

        for bad in ["../escape.md", "/tmp/jit-escape.md"] {
            let cfg = InvariantProjectionConfig {
                mode: Some(ProjectionMode::SeparateFile),
                target: Some(bad.to_string()),
                ..Default::default()
            };
            let err = project_invariants(&store, &cfg, &registry_with_two()).unwrap_err();
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

    #[test]
    fn test_default_config_targets_separate_jit_owned_file() {
        // REQ-03: the shipped default targets a separate jit-owned file.
        let cfg = InvariantProjectionConfig::default();
        assert_eq!(cfg.mode(), ProjectionMode::SeparateFile);
        assert_eq!(cfg.target(), ".jit/invariants.md");
    }
}
