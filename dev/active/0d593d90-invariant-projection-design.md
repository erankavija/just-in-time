# Invariant projection engine (REQ-06) — design note

Issue: 0d593d90 — Project invariants into a configurable doc target.

## Decisions

- **Config layer owns the target.** `[invariant_projection]` table →
  `Option<InvariantProjectionConfig>` on `JitConfig` (config.rs), mirroring the
  `[item_kinds]`/`[namespaces.*]` precedent. Fields: `mode`
  (`separate-file`|`region`, typed `ProjectionMode` enum), `target` (repo-relative
  path), `region-begin`/`region-end` delimiters. Every field is optional; absent
  values resolve to config-layer consts.
- **Default = separate jit-owned file (D3, REQ-03).** With no table at all,
  `InvariantProjectionConfig::default()` is separate-file mode targeting
  `.jit/invariants.md` (the const `DEFAULT_INVARIANT_PROJECTION_TARGET`). The
  default never touches existing docs.
- **No doc-filename literal in the engine (REQ-04).** The const default lives in
  the CONFIG schema; `validation/projection.rs` reads the resolved path/mode/
  delimiters only from the passed config. Scoped grep over the engine code path
  (projection.rs before `#[cfg(test)]`) finds no `*.md`/`*.txt`/filename literal.

## Engine (validation/projection.rs)

- `render_invariants_markdown(&InvariantRegistry) -> String` — PURE, deterministic
  (authored order), unit-testable; renders id, statement, kind, enforced-by.
- `splice_region(existing, rendered, begin, end) -> Result<String, ProjectionError>`
  — PURE; replaces only the bytes between the delimiters and byte-preserves the
  prefix (through `begin`) and suffix (from `end`) verbatim (REQ-01). Missing/
  out-of-order delimiters are typed errors, never a silent clobber.
- `project_invariants(store, repo_root, config, registry) -> Result<String, ProjectionError>`
  — the only I/O function. separate-file: render → `write_file_atomic` (REQ-02).
  region: `read_repo_file` (path-safe) → `splice_region` → `write_file_atomic`.

## Shared atomic writer (REQ-05)

Extracted `pub fn write_file_atomic` from the private `write_atomic`
(serialize.rs), with `# Examples`; all existing callers updated in the same
change. Projection writes reuse it — no duplicated atomic-write logic.

## CLI is out of scope

This issue delivers the engine as a public, ergonomic function. The
`jit invariant render` CLI that calls `project_invariants` is issue 52ad3b1f
(Wave 7).

## Example config

Default (implicit) — separate jit-owned file:

```toml
# no [invariant_projection] table → .jit/invariants.md, separate-file mode
```

Explicit separate-file:

```toml
[invariant_projection]
mode = "separate-file"
target = "docs/invariants.md"
```

Region mode (rewrite only the delimited block in an existing file):

```toml
[invariant_projection]
mode = "region"
target = "docs/reference/invariants.md"
region-begin = "<!-- jit:invariants:begin -->"
region-end = "<!-- jit:invariants:end -->"
```
