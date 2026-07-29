# Relative asset resolution for HTML documents — design

**Bug:** `5c060496` — relative asset paths in HTML decks fail to resolve
**Parent epic:** `94f873c8` (Docs Lifecycle Phase 2)

## Problem

HTML documents served through the `/raw` endpoints cannot load sibling assets via relative paths. The browser resolves `<img src="figures/fig4.svg">` against the iframe URL (`/api/issues/<id>/documents/<encoded-html>/raw`), producing a URL that matches no route. Concrete example: gf2 issue `6efb756b` deck references three relative SVG figures that all 404.

Two gaps cause this:
1. No route exposes sibling files of a linked HTML document.
2. Served HTML has no hint about where to resolve relative paths.

## Approach

Server-side fix, two coordinated changes. Purely additive — existing endpoints remain byte-faithful for non-HTML content, behaviour unchanged for decks that don't use relative paths.

### 1. Wildcard raw route

New route `GET /api/raw/*path` (axum wildcard).

```rust
.route("/raw/*path", get(get_raw_wildcard))
```

Handler `get_raw_wildcard`:

- Extract `path: String` from the wildcard capture.
- Extract `commit: Option<String>` from query string (same shape as existing `/raw` query params).
- Validate path (see Security below) — return 400 on rejection.
- Read bytes via `state.executor.read_path_bytes(&path, commit.as_deref())`.
- Build response with `Content-Type: infer_content_type(&path)`, `Content-Security-Policy: CSP_HEADER`, body = bytes.
- If `Content-Type == "text/html"` and no git commit is pinned, apply base-tag injection (see #2 below).

Uses the same byte-faithful machinery as `get_document_raw_by_path`. No new storage-layer work required — `IssueStore::read_path_bytes` already handles arbitrary paths with repo-root resolution and typed `PathReadError` (including `NotFound` → 404 via `path_read_error_status`).

### 2. Base-tag injection

New helper in `crates/server/src/routes.rs`:

```rust
/// Inject `<base href="...">` into an HTML document so relative paths resolve
/// correctly when the HTML is served from a URL whose directory structure
/// doesn't match the source file's location.
///
/// Returns the HTML unchanged if it already contains a `<base>` tag in `<head>`.
/// Inserts right after `<head>` (or `<head …attrs…>`), or before `</html>` if
/// no `<head>` is found.
fn inject_base_href(html: &str, base_href: &str) -> String;
```

Called only when:
- Response Content-Type is `text/html`
- The request has no `?commit=` pin (otherwise the base tag could point at a URL that serves working-tree bytes while the HTML is from a commit — confusing; defer for now)

Three callers update: `get_document_raw` (issue-scoped), `get_document_raw_by_path` (path-only), and `get_raw_wildcard`.

Base href computation: URL-encode each path segment of the HTML's parent directory, then format as `/api/raw/<encoded-parent-dir>/`. Trailing slash critical for HTML `<base>` semantics.

Example: HTML at `docs/presentations/deck.html` → base href `/api/raw/docs/presentations/` → `<img src="figures/fig4.svg">` resolves to `/api/raw/docs/presentations/figures/fig4.svg` → wildcard route serves it.

Skip injection when the HTML already has a `<base>` tag inside `<head>` — user-authored intent wins. Use a cheap substring search (`html[..head_close_idx].contains("<base")` with case-insensitive comparison) rather than full parsing.

### Security

Path validation for the wildcard route. Reject with HTTP 400 on any of:

- Path is empty.
- Path contains a `..` segment (split on `/`, check each component).
- Path starts with `/` (absolute path — axum may or may not strip it; be explicit).
- After repo-root resolution, canonicalized path does not start with the repo root. Guards against symlink escapes. Use `std::path::Path::canonicalize` via storage layer; if storage layer doesn't already do this, add the check here.

Tests must cover: `../etc/passwd`, `a/../../b`, `/absolute/path`, legit `docs/presentations/figures/fig4.svg`, and a filename containing `..` as part of its name (e.g. `foo..bar.txt` — should be accepted; only `..` as a whole segment is forbidden).

The existing `/raw` endpoints already rely on `IssueStore::read_path_bytes` for I/O; if that method's current implementation doesn't guard against symlink escapes, the wildcard route inherits that gap. Add a dedicated validator function `validate_repo_relative_path(path: &str) -> Result<(), ValidationError>` in routes.rs (or a small shared helper) that handles the string-level checks cleanly; leave canonicalization-based escapes as a follow-up if storage layer needs hardening.

## Files to modify

- `crates/server/src/routes.rs`:
  - Register `/raw/*path` in the router (near existing `/documents/raw`).
  - New handler `get_raw_wildcard`.
  - New helper `inject_base_href(html, base_href)`.
  - New helper `validate_repo_relative_path(path)` with structured error.
  - Update `get_document_raw` and `get_document_raw_by_path` to call `inject_base_href` when Content-Type is text/html and commit isn't pinned.
  - Tests for all of the above.

- Existing `CSP_HEADER` constant reused as-is — no CSP change.

- No storage-layer changes required.
- No web-side changes required — existing iframe src / "Open in new tab" URLs continue to work. The base tag injection is transparent to the client.

## Tests

### Unit

- `inject_base_href_inserts_after_head()` — `<html><head>…</head>…` gets `<base>` right after `<head>`.
- `inject_base_href_handles_head_with_attrs()` — `<head lang="en">`.
- `inject_base_href_no_head_inserts_before_body()` — document without explicit `<head>`.
- `inject_base_href_preserves_existing_base()` — HTML with an existing `<base>` tag returns unchanged.
- `inject_base_href_case_insensitive_head()` — `<HEAD>`, `<Head>` both handled.
- `inject_base_href_non_html_input()` — document that isn't recognizable HTML is returned unchanged (defensive).
- `validate_repo_relative_path` — accepts clean relatives, rejects `..` segments, rejects absolute paths.

### Integration (axum TestServer)

- `test_get_raw_wildcard_serves_svg()` — fixture with `docs/presentations/figures/fig4.svg`, GET `/api/raw/docs/presentations/figures/fig4.svg`, assert 200 + `Content-Type: image/svg+xml` + body bytes match.
- `test_get_raw_wildcard_html_injects_base_tag()` — fixture with HTML at `docs/presentations/deck.html`, GET `/api/raw/docs/presentations/deck.html`, assert body contains `<base href="/api/raw/docs/presentations/">`.
- `test_get_raw_wildcard_rejects_path_traversal()` — GET `/api/raw/../etc/passwd`, assert 400.
- `test_get_raw_wildcard_rejects_embedded_dotdot()` — GET `/api/raw/foo/../bar`, assert 400.
- `test_get_raw_wildcard_respects_commit_param()` — git-commit round-trip on a sample file (parallel to existing `/raw` commit tests).
- `test_get_document_raw_html_injects_base_tag()` — existing issue-scoped /raw, assert injected base href matches the HTML's parent directory.
- `test_get_document_raw_preserves_existing_base_tag()` — issue-scoped /raw fixture where the HTML already has a `<base>`, assert the response body is unchanged.
- `test_get_document_raw_non_html_bytes_unchanged()` — regression for the existing byte-faithful behaviour for non-HTML content.

### Extensions to existing tests

- Add `image/svg+xml` to `infer_content_type` (currently missing — new relevant extension now that figures matter). Similarly add `image/png` and `image/jpeg` defensively. Covered by extending `test_infer_content_type_other_extensions`.

## Verification plan

1. Local cargo: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` — all green.
2. Integration test suite covers all cases above.
3. E2E against gf2 (same pattern as story `abfd6016` close): `jit-server` serving `../gf2`, open issue `6efb756b` in the web UI. All three figures render inline within the reveal.js deck. Verified via:
   ```
   curl -sI http://localhost:3000/api/raw/docs/presentations/figures/fig4_crc_25_15_comparison.svg
   ```
   Returns 200 with `Content-Type: image/svg+xml`.
4. Regression E2E: issue `d4851c3d` modem-framework deck (no relative figures, CDN-only) still renders identically.

## Estimated scope

One implementation task under bug `5c060496`. ~150-200 LOC in routes.rs (handler, helpers, 10-12 tests), plus ~5 LOC for added MIME types. No cross-layer changes, no web changes, no DB schema changes. Expected to pass gates in one or two review cycles.

## Out of scope (follow-on)

- Rewriting `url(…)` in CSS and dynamic JS-loaded assets. Base-tag injection handles HTML-level element src/href natively; CSS-internal `url(…)` inside `<style>` blocks ALSO gets resolved relative to `<base href>`, so this is fully covered. Runtime-fetched JS assets also inherit base resolution. No explicit handling needed.
- Cross-deck navigation (link from one deck to another): works natively once wildcard route exists.
- Document author tooling to bundle decks into self-contained HTML.
- Hardening `IssueStore::read_path_bytes` against symlink escapes (track separately if needed; the routes-layer validator handles the common case).
