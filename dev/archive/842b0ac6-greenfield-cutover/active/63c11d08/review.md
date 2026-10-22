## Review: Remove the remaining domain re-export paths (63c11d08)

**Verdict:** PASS

### Gate status

Cargo CI passed in run 788882e7-e067-4b26-96bc-76afca3628d9: 4,588 tests, 58 doctests, no skips or ignored tests, formatting, zero-warning Clippy, and build budgets. Independent code review a3462790-a2bf-46bd-afc4-360f4d392c5e passed with zero findings.

### Prior-findings regression table

| Round | Finding | Status at HEAD | Evidence |
|---|---|---|---|
| Lead 1 | Output/profile aliases leave duplicate domain paths | Closed | output.rs private imports; profile/mod.rs no domain re-exports; consumers use owning modules (2426bacea) |
| Cargo 1 | Generated event-reference source header is stale | Closed | Authoritative regeneration abbb84126; all four projection checks and full suite pass |

No prior failed code-review runs exist. The table records lead and test findings so their resolution remains discoverable.

### Success criteria

- [x] REQ-01: domain/mod.rs re-exports only its private types module; other items use owning module paths.
- [x] REQ-02: lib.rs exposes no domain, graph, or query re-exports.
- [x] REQ-03: output/profile duplicates are removed; consumer compilation and workspace-wide re-export review confirm the canonical paths.
- [x] REQ-04: domain module documentation names only domain submodules.

### Stale-narrative sweep

Current source, doctest and adopter-document paths converge. The steward authorized amending the df129d9e plan/manifest and f7f01654/650e6a56 descriptions to supersede the old output-envelope location. Historical investigation and overlap notes retain their historical context.

### Deferred-items audit

The task produced no design document or deferred implementation surface. The shared plan's domain-import-path contract is satisfied; its unrelated explicit non-goals are unchanged.

### Holistic findings

The independent follow-up audit confirmed the alias correction. The epic's two stale comments were resolved separately at 9ccf7d411. Wire/schema behavior is unchanged.
