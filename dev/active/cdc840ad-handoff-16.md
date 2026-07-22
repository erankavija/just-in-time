# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 16

**Date:** 2026-07-22T21:39:09+03:00
**Session number:** 16
**Prior handoffs:** `dev/active/cdc840ad-handoff.md` through `dev/active/cdc840ad-handoff-15.md`

## Current state

- Epic `cdc840ad`: wave 5 of 9.
- Active child `49adf23b`: implementation and fixture corrections are committed; issue remains `in_progress`.
- Rework count: 2. One open `rework-exceeded` escalation blocks further source edits.
- Worktree is clean after committing authoritative cargo-ci history at `6a6c64a3`.

## Accepted work

- Final legacy publisher deletion: `1a661786`, net 381 lines deleted.
- Shared selective-example clarification: `65d7b7f1`.
- Four canonical doctest preimages: `399d6f76`.
- Independent semantic and architecture reviews pass.
- Exact seven-writer absence scan is empty; `IssueStore` is read/query/session-control only.
- Full workspace passes, including 3,754 cargo-ci tests and 62 doctests.
- MCP previously passed 54 unit and 9 integration tests.

## Gate status

- `cargo-ci`: passed at run `49745f2d-f73b-4889-980a-016ef054243a`.
- `code-review`: pending; checker did not start.
- `mcp-ci`: pending.
- `docs-mechanical`: pending.

The first cargo-ci run failed only on two disposable incremental cache directories; they were removed and the rerun passed every subcheck. Both run records are committed.

## Current blocker

The repository registry configures `prompt_file = "./scripts/code-review-prompt.md"`. Gate capture routes that configured repository-relative string directly to `VirtualPath::worktree`; canonical `RootRelativePath` correctly rejects a `.` path component. The adapter then masks the typed lexical error as “resolves outside the repository.” Physical path inspection confirms the prompt is inside this repository.

The narrow fix is at the configuration-to-virtual-path adapter, not in `RootRelativePath` and not in `.jit/gates.toml`: strip leading `./` components before canonical construction, while retaining all rejection of absolute paths, parent traversal, interior dot components, backslashes, colons, controls, and empty paths. Add focused repository-style prompt capture/evaluation coverage.

## Required decision

The execution-lead rework limit is exhausted. Recommended option: authorize the narrow adapter normalization above with a counter reset, then rerun code-review and the remaining gates. Alternatives are manual takeover or rejection, which blocks waves 6–9.

## Traps

- Do not weaken `RootRelativePath` canonicality.
- Do not edit repository-local `gates.toml` to hide the adapter defect; shipped and adopter configurations may use leading `./`.
- Do not map every lexical error to “outside repository”; preserve the security rejection while making configured safe spelling canonical.
- Do not rerun cargo-ci unless source changes require it; it already passed and its checker is expensive.
- Preserve all prior handoff traps.
