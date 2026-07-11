# Review — Root and component documentation audit (36d5451e)

**Verdict:** FAIL

## Gate status

- `repo-validate`: passed (`1c62b901-66af-4035-9223-140af073d879`).
- `docs-mechanical`: passed with the declared root/component footprint (`b45ee673-7b03-4029-bdb9-0e6dc65a2107`).
- `doc-review`: failed in two overlapping recorded runs: `97b1a203-c4be-401b-a755-7e403b6d23c0` (seven findings) and `d525b0c2-75f4-4088-890e-02862407b372` (ten findings).

## Prior-findings regression table (Tier 1.5)

| Round | Finding | Status at HEAD | Evidence |
|---|---|---|---|
| R1 | Component images use unpublished `:latest` tags | closed | `INSTALL.md:91-150` uses `:main` for components and keeps all-in-one `:latest`. |
| R1 | `jit-mcp-server --version` is not an executable version check | closed | `INSTALL.md:185-195` uses `which jit-mcp-server`. |
| R1/R2 | Inert `validation.strictness` is presented as active | closed | `README.md:215-234` no longer presents the field. |
| R1 | Rust 1.80 MSRV is unsupported | closed | `INSTALL.md:159-163` states only the stable-toolchain policy and cites CI. |
| R2 | MCP success uses a fabricated `success`/`data` envelope | closed | `mcp-server/README.md:219-230` describes `CallToolResult` modes from `index.js:139-158`. |
| R2 | System-resource figures are unsupported | closed | `INSTALL.md:244-249` contains only functional optional dependencies. |
| R3/R4 | New MCP, archive, web-serving, and Docker documentation defects | open | Required changes below; no previous finding regressed. |

## Success criteria (Tier 2)

- [ ] REQ-01 and REQ-04 — unmet: the two current review runs identify 17 source-contradicting claims within the declared footprint.
- [x] REQ-02 — the scoped mechanical gate reports links, anchors, source citations, and item citations resolved.
- [x] REQ-03 — stale-narrative sweep and the reviewers found no remaining future/legacy narration after the earlier project-status finding was addressed.
- [x] REQ-05 — no diagram-shaped non-Mermaid block was identified.
- [x] REQ-06 — missing projection facts remain cited and categorized for the Group-C filing task in the audit notes.

## Stale-narrative sweep (Tier 2.5)

No stale task-term future/pending references in the scoped adopter documents. The planning document's follow-up references are an explicit Group-C non-goal, not adopter prose.

## Deferred-items audit (Tier 2.75)

- `dev/active/2d109173-plan.md`: its follow-up/open-gap markers explicitly delegate missing projection surfaces to Group C; they are legitimate non-goals.
- `dev/active/36d5451e-audit-notes.md`: the MSRV, inert strictness key, default ports, and MCP limits are explicitly classified as Group-C follow-ups; the unsupported system-resource figures are explicitly removed, not deferred.

## Holistic findings (Tier 3)

The work remains within the declared root/component footprint. The source-backed findings below show that the remaining deployment and MCP setup narrative is not yet coherent across `README.md`, `INSTALL.md`, `mcp-server/README.md`, and `web/README.md`.

## Required changes

1. `README.md`: qualify `jit-server` as serving UI assets only when embedded at build time or supplied through `--web-dir`.
2. `README.md`: replace the invalid archive example (`features` category/unmanaged `design.md`) with a valid generic or configuration-matched example.
3. `README.md`: remove future-facing project-status narration and hard-coded release-series claim; state only present compatibility behavior.
4. `mcp-server/README.md`: limit generated-tool coverage to schema leaf commands rather than every CLI command.
5. `mcp-server/README.md`: say the MCP surface reflects the schema loaded at startup and requires restart after CLI changes.
6. `mcp-server/README.md`: state direct `execFile` execution and inherited host environment, not shell execution.
7. `mcp-server/README.md`: say its advertised/logged version comes from the loaded `jit --schema` version.
8. `INSTALL.md`: make the source-build MCP test recipe place `target/release` on `PATH` before `npm test`.
9. `mcp-server/README.md`: make basic server startup require a built or installed `jit` executable on `PATH`.
10. `mcp-server/README.md`: replace obsolete GitHub Copilot configuration paths and verification command with the current official CLI workflow, verified against GitHub's current documentation.
11. `INSTALL.md`: replace Vite/static-serving guidance that leaves `/api` unroutable with a supported same-origin workflow or an explicit reverse-proxy requirement.
12. `web/README.md`: give a local-development path whose `/api` requests are routable, or direct users to the supported same-origin built-asset workflow.
13. `INSTALL.md`: make the individual Web container example provide nginx's required `api` hostname/network, or direct readers to Compose.
14. `INSTALL.md`: use `--entrypoint sh` for the CLI image interactive-shell command.
15. `INSTALL.md`: make the all-in-one example provide nginx's `api` hostname for the locally started `jit-server`, or use a valid alternative.

Items 1/7, 8/9, and 11/12 overlap across the two runs but must each be verified at every cited surface. The rework agent must read both complete gate records and produce a resolution table covering every numbered finding.
