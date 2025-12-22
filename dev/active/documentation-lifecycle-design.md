# Documentation Lifecycle and Knowledge Capture — Design Plan

**Note:** This design was created before the documentation reorganization (Issue 165cf162). References to paths have been updated to reflect the current implementation.

This document consolidates and extends the existing strategy for documentation lifecycle management in JIT, adds first‑class support for binary assets, and generalizes document processing so projects are not limited to Markdown.

Related references:
- `documentation-organization-strategy.md` (implemented structure - Phase 1)
- `documentation-lifecycle-strategy.md` (superseded early strategy)
- `../vision/knowledge-management-vision.md` (vision and CLI outline)
- `crates/jit/src/commands/document.rs` (existing document read/history APIs)

## 1) Goals

- Keep `dev/active/` focused on active work; `docs/` for permanent product documentation.
- Preserve complete historical context in `dev/archive/` without link rot.
- Make snapshots portable and reproducible, including all linked assets.
- Support multiple documentation formats (not Markdown‑only) via pluggable format adapters.
- Provide a configurable policy per project (manual by default; optional automation).
- Be safe by default (validate link integrity; never mutate repo during snapshots).

## 2) Scope

- Document lifecycle: authoring → active → recently completed → archived.
- Assets lifecycle: discovery, classification (per‑doc vs shared), movement/rewrites, inclusion in snapshots.
- Snapshot export: `.jit` state + linked documents + assets + manifest, non‑mutating.
- Format‑agnostic asset/link handling via adapters.

Non‑goals:
- WYSIWYG authoring.
- Rendering/publishing pipelines.
- Complex cross‑repo document dependency management.

## 3) Current State Summary (Updated)

- **Active designs:** `dev/active/` (linked to in-progress issues)
- **Archived docs:** `dev/archive/<category>/...` with link preservation via JIT references
- **Product docs:** `docs/` (permanent, user-facing, never archived)
- **CLI/API groundwork:** `jit doc add/list/remove/show`, `read_document_content`, `get_document_history`
- **Strategy:** Manual archival initially (Phase 1 complete)

## 4) Requirements

Functional:
- Configurable archival policy and categories.
- Asset discovery for each doc; stable handling of binary assets (images, diagrams).
- Move/Archive operations that preserve link integrity or safely rewrite links.
- Snapshot export including all linked docs and assets with verification manifest.
- Multi‑format support (Markdown, AsciiDoc, reStructuredText, MDX, etc.) via adapters.

Non‑functional:
- Safety: never corrupt links; provide dry‑run plans; fail fast on ambiguities.
- Portability: snapshots self‑contained or clearly annotated when external/LFS content is missing.
- Extensibility: new formats and policies without core rewrites.
- Performance: IO‑bound operations; acceptable for typical repos.

## 5) Configuration

Add `[documentation]` to `config.toml`. Manual default, with optional automation and asset policies.

**Current implementation (Phase 1):**
```toml
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]
```

**Proposed extension (Phase 2 - Asset Management & Automation):**
```toml
[documentation]
# Base configuration (implemented in Phase 1)
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

# Archival automation (Phase 2)
mode = "manual"        # manual | release | done
retention_releases = 2 # used by manual/release modes

# Categories (subdirs under archive_root) - already created in Phase 1
categories.design   = "features"
categories.analysis = "bug-fixes"
categories.refactor = "refactorings"
categories.session  = "sessions"
categories.study    = "studies"

# Safety & UX (Phase 2)
strictness = "strict" # strict | loose | permissive
block_if_linked_to_active_issue = true
permanent_label = "permanent"

# Asset management (Phase 2)
[documentation.assets]
mode = "mixed"            # per-doc | shared | mixed
per_doc_suffix = "_assets"
shared_roots = ["docs/diagrams/", "dev/diagrams/"]

external.policy = "exclude"   # exclude | download | allow
lfs.policy = "allow-pointers" # require | allow-pointers | prefer-working-tree

# Format adapters (Phase 2)
[documentation.formats]
markdown = true
asciidoc = true
restructuredtext = false
mdx = false
```

## 6) CLI Surface

- `jit doc add <issue-id> <path> [--type design] [--scan-assets]`
  - Registers document and (by default) discovers referenced assets.
- `jit doc list <issue-id>`
- `jit doc remove <issue-id> <path>`
- `jit doc show <issue-id> <path> [--at <commit>]`
- `jit doc assets list <issue-id> <path>`
- `jit doc assets collect <issue-id> <path> [--dest <dir>] [--copy|--move]`
  - Normalize a doc to a per‑doc asset folder; update links.
- `jit doc check-links [--scope all|issue:<id>] [--fix]`
  - Validate link resolvability; optionally rewrite to normalized layout.
- `jit doc archive <path> [--issue <id>] [--type <design|...>] [--rewrite-links]`
  - Move doc (and per‑doc assets) to archive. Refuse if links break unless `--rewrite-links`.
- `jit doc sweep [--since-release <tag>|--since-date <YYYY-MM-DD>] [--dry-run] [--yes]`
  - Batch archive candidates per policy.
- `jit doc policy show`
  - Show resolved policy and format adapters.

Snapshots (non‑mutating):
- `jit snapshot export [--out <path>|--stdout] [--format tar|zip|dir] [--scope all|issue:<id>|epic:<id>] [--at <commit>|--working-tree] [--committed-only] [--download-external-assets] [--rewrite-links-in-snapshot] [--require-lfs]`

## 7) Data Model Extensions

Extend the document reference stored in issue metadata:

```json
{
  "path": "dev/active/webhooks.md",
  "commit": "abc123",                   // optional
  "doc_type": "design",
  "format": "markdown",                 // adapter id
  "assets": [
    {
      "path": "dev/active/webhooks_assets/flow.png",
      "shared": false,
      "hash_sha256": null,
      "mime": "image/png"
    }
  ]
}
```

Notes:
- `format` is resolved by adapter detection (extension, frontmatter, or content probe).
- `assets` can be refreshed (re‑scanned) on demand to reflect edits.

## 8) Format‑Agnostic Architecture

Introduce a trait for format adapters and a registry within `jit`:

```rust
pub trait DocFormatAdapter: Send + Sync {
    fn id(&self) -> &'static str; // "markdown", "asciidoc", ...
    fn supports_path(&self, path: &str) -> bool;
    fn detect(&self, path: &str, content: &str) -> bool;

    // Parse and extract referenced asset paths (repo-relative)
    fn scan_assets(&self, doc_dir: &std::path::Path, content: &str) -> anyhow::Result<Vec<String>>;

    // Rewrite links in content given a mapping (old -> new)
    fn rewrite_links(&self, content: &str, mapping: &[(String, String)]) -> anyhow::Result<String>;
}

pub struct AdapterRegistry {
    adapters: Vec<Box<dyn DocFormatAdapter>>,
}

impl AdapterRegistry {
    pub fn resolve(&self, path: &str, content: &str) -> Option<&dyn DocFormatAdapter> { /* ... */ }
}
```

- Built‑in adapters: Markdown (CommonMark subset), AsciiDoc, reStructuredText (opt‑in), MDX (opt‑in).
- Detection: Prefer extension mapping; fall back to light content probe.
- Config toggles enable/disable adapters; first match wins.

Adapter responsibilities:
- Asset scanning: image/link syntaxes for that format.
- Link rewriting: update textual references when doc or assets move.

Fallbacks:
- Unknown format: treat as opaque; no rewrite; archival allowed only if link resolution is provably preserved (e.g., root‑relative links) or user opts `--rewrite-links` with a specific adapter.

## 9) Asset Management

Authoring conventions:
- For a doc `dev/active/feature.md`, use per‑doc assets under `dev/active/feature_assets/` and link relatively: `./feature_assets/diagram.png`.
- For shared assets, prefer stable repo‑root‑relative links: `/docs/assets/shared/logo.png`.

Classification:
- Per‑doc assets: only referenced by a single doc; co‑move with the doc.
- Shared assets: referenced by multiple docs; remain in place; avoid rewrites.

Archival:
- Per‑doc assets move with the doc to `dev/archive/<category>/...`.
- Shared assets remain; links must be root‑relative or safely rewritten.
- Safety: refuse archival if relative shared links would break (unless `--rewrite-links`).

Checks and tooling:
- `jit doc assets list` to inspect.
- `jit doc check-links` to validate across the repo state and proposed operations.
- `jit doc assets collect` to normalize older docs into per‑doc layout.

External and LFS:
- External URLs: excluded by default; `--download-external-assets` can vendor them into a snapshot with source noted.
- LFS pointers: behavior per config (`require`, `allow-pointers`, `prefer-working-tree`); warn or fail accordingly.

## 10) Snapshot Export

Behavior:
- Read `.jit/` state and all linked documents in scope.
- For each doc, include exact content at `--at <commit>` or `HEAD` (or working tree), using `read_document_content`.
- Include assets:
  - Use tracked `assets[]`; re‑scan as a fallback to detect drift (warn).
  - Deduplicate by content hash.
  - Preserve original relative paths; mirror repo layout in the snapshot by default.

Options:
- `--format dir|tar|zip`
- `--committed-only`: refuse uncommitted assets/docs.
- `--rewrite-links-in-snapshot`: optionally rewrite links to snapshot‑local canonical paths.
- `--download-external-assets`: vendor HTTP(S) assets into `snapshot/assets/external/` with provenance.
- `--require-lfs`: fail if LFS content unavailable.

Manifest (example):

```json
{
  "version": "1",
  "created_at": "2025-12-21T12:00:00Z",
  "repo": "erankavija/just-in-time",
  "scope": "all",
  "at": "HEAD",
  "issues_count": 123,
  "documents_count": 42,
  "documents": [
    {
      "path": "dev/active/webhooks.md",
      "format": "markdown",
      "source": {"type": "git", "commit": "abc123", "blob_sha": "deadbeef"},
      "hash_sha256": "…",
      "assets": [
        {
          "path": "dev/active/webhooks_assets/flow.png",
          "source": {"type":"git","commit":"abc123","blob_sha":"…"},
          "hash_sha256": "…",
          "mime": "image/png",
          "shared": false
        }
      ]
    }
  ],
  "link_policy": "preserve",
  "external_assets_policy": "exclude",
  "lfs_policy": "allow-pointers"
}
```

## 11) Algorithms and Invariants

Archival move plan:
1. Resolve doc’s adapter; scan assets.
2. Classify assets:
   - If referenced by >1 doc (global index), mark shared.
   - Else per‑doc.
3. Compute destination path: `${archive_root}/${category}/${relative_doc_path}`
4. Validate links:
   - Per‑doc assets: moving both preserves relative links → OK.
   - Shared assets: root‑relative links → OK; otherwise require rewrite or normalization.
5. If rewrite needed and allowed:
   - Compute mapping and run adapter.rewrite_links(content, mapping).
6. Atomically:
   - Move files/dirs; update issue metadata paths; log `document_archived`.

Snapshot plan:
1. Enumerate docs in scope from `.jit/issues`.
2. For each doc:
   - Resolve content and commit via `read_document_content`.
   - Resolve and read assets; compute hashes.
3. Build manifest; write payload tree (mirroring repo layout by default).
4. Package to requested format.

Invariants:
- No path escapes outside repo root.
- After archival, `jit doc show` must resolve the doc.
- Snapshot never writes to the repo.

## 12) Storage and Events

- Extend issue JSON to include `format` and `assets[]`.
- Append events to `.jit/events.jsonl`:
  - `document_assets_scanned`
  - `document_archived`
  - `document_links_rewritten`
  - `snapshot_exported`

## 13) Testing

Unit:
- Asset scanning per adapter (Markdown, AsciiDoc).
- Link rewriting with edge cases (URLs, anchors, data URIs).
- Classification (per‑doc vs shared).

Integration:
- Archive a doc with per‑doc assets → links preserved.
- Archive a doc with shared assets (root‑relative) → preserved.
- Archive refusal when relative shared links would break; honored with `--rewrite-links`.
- Snapshot with `--committed-only` uses git blobs; manifest hashes verified.
- LFS pointer handling across policies.
- Format fallback: unknown format → archival only when safe, else require explicit adapter.

Property‑based:
- Path normalization and mapping do not escape repo root.
- Rewriting followed by reading resolves the same targets.

## 14) Migration Plan

- Default `mode="manual"`; no changes without explicit commands.
- Add `jit doc assets collect` to normalize historical docs on demand.
- Pilot automation with `mode="release"` on a subset (`--scope epic:<id>`).
- Document authoring conventions in `docs/README.md`.

## 15) Open Questions

- Should we allow external adapter executables (plugin protocol) in addition to built‑ins?
- Do we need pre‑export hooks for generating derived assets (e.g., Graphviz)? Likely a later phase.
- How to index cross‑doc references for “shared asset” detection efficiently at scale? Start naive, optimize with a repo‑level cache.

## 16) Phased Delivery

Phase 1 (Foundations)
- Adapter registry + Markdown adapter.
- Asset scanning and metadata extension.
- `doc assets list`, `doc check-links`.
- Manual `doc archive` with per‑doc asset co‑move.
- Snapshot export (dir|tar) with manifest; committed‑only mode.

Phase 2 (Ergonomics)
- `doc assets collect` and link rewriting.
- `doc sweep` with dry‑run and confirmations.
- AsciiDoc adapter; LFS handling.

Phase 3 (Advanced)
- External asset download option for snapshots.
- MDX/reST adapters (opt‑in).
- Performance improvements and cache for shared asset detection.

## 17) Developer Notes

- Reuse `read_document_content` and `get_document_history` for git‑accurate reads.
- Add binary readers:
  - `read_binary_from_git(repo, path, ref) -> Vec<u8>`
- Define typed errors with `thiserror` for preconditions and link validation.
- Keep snapshot writers isolated from repo state; use temp dirs + atomic file moves for archives.
