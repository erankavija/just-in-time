# Multi-format document rendering in the web UI — design

**Story:** `abfd6016` — Multi-format document rendering in web UI (reveal.js / HTML first)
**Parent epic:** `94f873c8` — Docs Lifecycle Phase 2 (addresses "Limited format support (Markdown only)")
**Child tasks:** `75b1ccb8` (server), `42bd50ce` (registry), `eeafaa78` (HtmlRenderer), `85bd1288` (HtmlAdapter)

## Problem

`web/src/components/Document/DocumentViewer.tsx` pipes every issue-linked document through `<ReactMarkdown>` unconditionally. For `.html` artifacts (notably reveal.js presentations that users already attach to issues — e.g. `../gf2` issue `d4851c3d`'s `docs/presentations/d4851c3d-modem-framework.html`), the server falls back to `content_type: text/plain` (`crates/server/src/routes.rs:340-352`) and the viewer renders the raw HTML source as markdown.

The data model is already prepared: `DocumentReference` has `format` and `doc_type` fields (`crates/jit/src/domain/types.rs`), and `AdapterRegistry` (`crates/jit/src/document/adapter.rs`) is explicitly a plugin system with `with_builtins()` — currently only `MarkdownAdapter` is registered.

## Approach

A client-side **renderer registry** keyed off server-inferred `content_type`, plus a server raw-bytes endpoint so an `<iframe>` can load HTML artifacts directly. Scope is the framework + one concrete non-Markdown renderer (HTML/reveal.js); PDF / AsciiDoc / Jupyter follow the same pattern in later stories.

### Server (task 75b1ccb8)

- Extend `infer_content_type()` to recognise `.html` / `.htm` → `text/html`.
- New route `GET /api/issues/:id/documents/:path/raw` — reuses `executor.read_document_content()` for path + commit resolution, returns file bytes with correct `Content-Type` set on the response (not JSON-wrapped).
- Symmetric `GET /api/documents/raw?path=…` for path-only loads.
- Add a CSP response header permitting `https:` so reveal.js's jsdelivr CDN works through the iframe.

### Web — renderer registry (task 42bd50ce)

- Introduce a `DocumentRenderer` interface: `{ id; match(content, ref); Component }`.
- New registry at `web/src/components/Document/renderers/index.ts`. Ordered, first-match-wins, catch-all MarkdownRenderer last.
- Extract current ReactMarkdown body verbatim into `renderers/MarkdownRenderer.tsx` (no behaviour change).
- `DocumentViewer.tsx` becomes a thin dispatcher: fetch content as today, delegate body rendering to the matched renderer, keep the header/footer/history scaffolding.
- Selection: `content_type === 'text/html'` → `HtmlRenderer`, else → `MarkdownRenderer`.

### Web — HtmlRenderer (task eeafaa78)

- New component `renderers/HtmlRenderer.tsx`.
- Full-height `<iframe src={rawUrl} sandbox="allow-scripts allow-same-origin allow-popups" />` pointing at the server raw endpoint (with `?commit=…` when pinned).
- `allow-scripts` is required for reveal.js; `allow-same-origin` lets the deck fetch co-located assets; `allow-popups` lets in-deck links open.
- Header toolbar: label, path, "Open in new tab" button (`target="_blank" rel="noopener"`), content-type badge.
- Skip search highlighting and the history panel (not meaningful through an iframe).
- Adds `getRawDocumentUrl(issueId, path, commit?)` helper to `web/src/api/client.ts`.

### Core — HtmlAdapter (task 85bd1288)

- Add `HtmlAdapter` to `crates/jit/src/document/adapter.rs` implementing `DocFormatAdapter`.
- `id()` → `"html"`; `supports_path()` → `.html` / `.htm`; `detect()` → `<!DOCTYPE html` prefix or `<html` substring.
- `scan_assets()` — coarse regex extraction of `src`/`href` values; exclude external URLs, `mailto:`, anchor-only. Acceptable to be approximate for v1.
- Register in `AdapterRegistry::with_builtins()`.
- Does NOT migrate existing stored `DocumentReference.format` values. Retroactive fix-up is a separate follow-up.

## Reuse

- `DocumentReference.format` / `doc_type` fields already exist — no schema change.
- `AdapterRegistry` already accepts multiple adapters — just register one more.
- `executor.read_document_content()` already handles git-commit resolution and filesystem fallback — the new raw endpoint reuses it.
- `../gf2/docs/presentations/*.html` are real-world reveal.js fixtures for end-to-end validation.

## Acceptance (story-level)

- A reveal.js deck linked to a gf2-style issue renders inline in the web UI with working slide navigation and reveal.js fullscreen.
- "Open in new tab" opens the raw HTML standalone at the correct `Content-Type`.
- Existing Markdown documents render byte-identically to before — no regression in content, search highlighting, or history panel.
- `jit document add <issue> <file.html>` records `format: "html"` in the stored `DocumentReference`.
- Renderer registry is extensible: adding a new format in a future story is mechanical (one registry entry + one component), no touches to `DocumentViewer.tsx`.

## Dependency DAG

```
story abfd6016
  ├── task 75b1ccb8  server: content-type + raw endpoint
  ├── task 42bd50ce  web: renderer registry
  ├── task eeafaa78  web: HtmlRenderer                   [deps: 75b1ccb8, 42bd50ce]
  └── task 85bd1288  core: HtmlAdapter
```

Tasks `75b1ccb8`, `42bd50ce`, `85bd1288` run concurrently in wave 1. Task `eeafaa78` runs in wave 2 after wave 1 completes.

## Out of scope (follow-on stories under the same epic)

- PDF, AsciiDoc, Jupyter, ODP, image viewers.
- Retroactive migration of existing `.html` documents whose stored `format` is wrong.
- Link rewriting inside HTML documents (Phase 1 adapter also left this unimplemented for Markdown).
- Authoring flows (creating reveal.js decks inside JIT).
