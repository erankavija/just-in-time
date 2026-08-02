# JIT Development Documentation

Development documentation for Just-In-Time (JIT), written for contributors working on JIT itself.

## Documentation Domains

**Product documentation** — [docs/index.md](../docs/index.md). What JIT is and how to use it, for adopters.

**Development documentation** — this directory (`dev/`). How JIT is built: architecture, investigations, and the working artifacts of open issues.

---

## Areas Inside the Development Root

`dev/` is this repository's development root. Which areas it holds, which of them
organize their artifacts one directory per issue, and what archival does to the
artifacts in each are declared in the `[documentation]` table of this
repository's `.jit/config.toml` — this project's own policy rather than a
shipped default. `jit config get documentation` prints the table in force, and
the configuration reference defines what its declarations mean:

- [Development-area classification](../docs/reference/configuration.md#development-area-classification) — what an area's managed or permanent class states about archival.
- [Issue artifact directories](../docs/reference/configuration.md#issue-artifact-directories) — how the directory an issue owns inside an issue-scoped area is derived.

## Adding a Document

1. Pick the area from the configured table.
2. In an issue-scoped area, let the tool name the issue's directory instead of
   composing it ([`jit doc dir`](../docs/reference/cli-commands.md#jit-doc-dir)):

   ```bash
   mkdir -p "$(jit doc dir <issue-id> <area>)"
   ```

3. Link the document to its issue, which is what puts it in reach of archival:

   ```bash
   jit doc add <issue-id> <path>
   ```

4. Keep assets and links portable — [authoring-conventions.md](authoring-conventions.md)
   gives the patterns, and `jit doc check-links` validates them.

[`jit doc conformance`](../docs/reference/cli-commands.md#jit-doc-conformance)
reports artifacts sitting outside the directory their owning issue owns. It is
advice: it writes nothing, blocks no transition, and a listed artifact stays
resolvable where it is.

## Archival

A document is archived with the container that owns it.
`jit archive container <id>` plans every artifact linked to a container and its
hierarchy descendants; `jit archive document <path>` targets one document and
the bundle it reaches. Both are read-only previews until `--execute`, which
recomputes the plan under the repository write guard and refuses an ineligible
one. A container must be effectively terminal, and a successful execution
retires it into the `Archived` state.

A container's artifacts land in one directory under the configured archive root,
named from the container's short id and the slug its membership label resolves
to. Beneath that directory each artifact keeps its repository-relative source
path, so a document archived out of an area is found again under the same area
name inside the container's directory.
[Archive planning and execution](../docs/reference/cli-commands.md#archive-planning-and-execution)
specifies the planner's decisions, the destination rule, and what execution
guarantees.

The archive root also holds directories filed by hand. They stay as they are;
planned destinations are named from the container.

---

## Regenerating a Generated Artifact

Some files in this checkout are generated and committed: reference pages
projected from engine declarations, and marked regions rendered from a packaged
authority. Each one is guarded by an assertion that fails when the committed
bytes differ from what its source renders, and the failure names the command
that repairs it.

Every such command is one script under `scripts/`, named `generate-` and the
artifact, taking no arguments:

```bash
./scripts/generate-events-reference.sh
```

A run prints one `updated: <path>` line per file it rewrote and then one `OK: …`
summary, so an unchanged artifact is distinguishable from a repaired one. It
exits 0 when the artifact holds its rendered values, 1 when the render or the
publication failed, and 2 when the run was refused or the environment could not
support it. Nothing here runs during `cargo test`: the assertions read, and only
these entry points write.

The set of artifacts, each one's target and entry point, and where each render
lives are declared in
[crates/jit/src/generated_artifacts.rs](../crates/jit/src/generated_artifacts.rs);
a test there holds every declared entry point against the checkout. A generator
whose values come from library code renders through the `regenerate` example,
which that declaration names; one whose values can only be read out of an
already-installed binary renders inside its own script. Which of the two applies
follows from where the artifact's authority lives, and each script's header says
so.

Regions declared as `[projection.*]` in this repository's `.jit/` configuration
reach their targets through `jit project render`, the product command an adopter
runs the same way; [../AGENTS.md](../AGENTS.md) states the rule for editing
them.

---

## Assembling the Workflow Package

This repository's workflow package is a directory: a manifest, the assets it
declares, and a managed-region source. Most of those assets name a repository
file as their target and carry that file's bytes, so the repository file is the
authority and the tree is produced from it:

```bash
./scripts/assemble-package.sh target/package/jit-dogfood
```

The entry point takes the destination and draws each declared source from the
side that owns it — a live asset from the repository file its declaration
targets, everything else from the checked-in sources under
[profiles/jit-dogfood](../profiles/jit-dogfood). Each run publishes a freshly
staged tree, so a source the manifest stops declaring is absent from the next
one. It exits 0 when the tree is assembled at the destination, 1 when the
assembly or the publication failed, and 2 on a usage or environment error.

It is not one of the artifacts above: what it writes is not committed, so it
carries no drift assertion. The destination is whichever path the caller names —
the example above names one under `target/`, which this repository ignores — and
a run replaces it whole. No build consumes what a run produces; the run reads its
own result back to validate it, which is a check rather than a dependency, so
assembling during a build would make every build do work no build consumes, and
the directory watching a build step needs is what once relinked every test
target on an unchanged rebuild. The render lives in
[crates/jit/src/profile/package_assembly.rs](../crates/jit/src/profile/package_assembly.rs)
and reads the manifest through the crate's own package model, which leaves the
manifest one reader.

---

## Key Documents

### Architecture
- [core-system-design.md](architecture/core-system-design.md) — start here for the system overview
- [web-ui-architecture.md](architecture/web-ui-architecture.md) — web UI design
- [graph-filtering-architecture.md](architecture/graph-filtering-architecture.md) — graph query design
- [cli-and-mcp-strategy.md](architecture/cli-and-mcp-strategy.md) — CLI and MCP integration

### Vision
- [9db27a3a-charter.md](vision/9db27a3a-charter.md) — the v1.0 charter and its decision log
- [knowledge-management-vision.md](vision/knowledge-management-vision.md) — knowledge management direction

### Reference
- [TESTING.md](TESTING.md) — testing strategy and focused test commands
- [authoring-conventions.md](authoring-conventions.md) — asset and link patterns that survive archival
- [workflow-contract.md](workflow-contract.md) — how the committed GitHub workflows are verified and how their action pins are updated
- [architecture-pitfalls.md](studies/architecture-pitfalls.md) — common pitfalls
- [clippy-suppressions.md](studies/clippy-suppressions.md) — documented suppressions

---

## For Contributors

**Getting started:**
1. Read [../AGENTS.md](../AGENTS.md).
2. Review [architecture/core-system-design.md](architecture/core-system-design.md).
3. Use `jit query available` to find work.

**Agent surfaces:** [docs/tutorials/quickstart.md](../docs/tutorials/quickstart.md)
is the ten-minute introduction;
[docs/reference/cli-commands.md](../docs/reference/cli-commands.md#mcp-tools-reference)
covers the MCP tool surface for agents.

**Review policy ownership:** Treat applicable `AGENTS.md` files as canonical
engineering prose and configured addressable-item sources as canonical for
registry or issue policy. Repository-specific review prompts describe how to
discover and judge that policy; they do not duplicate it. The reviewer emits
human-readable findings, a structured findings block, and a terminal verdict.
The shared AI-review wrapper stays tool-agnostic: it transports the prompt and
context and determines its checker exit code from the terminal verdict. jit
parses and persists the structured findings block. When a qualified item
governs a finding, record its resolved ID in the finding's optional
`references` array so the review remains traceable. See [Custom Gates](../docs/how-to/custom-gates.md#ground-a-repository-review-in-canonical-policy)
for the integration pattern.
