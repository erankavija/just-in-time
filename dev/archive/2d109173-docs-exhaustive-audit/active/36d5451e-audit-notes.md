# Audit notes — Root and component documentation audit (36d5451e)

Footprint: `README.md`, `INSTALL.md`, `mcp-server/README.md`, `web/README.md`,
`docs/index.md`, `docs/README.md`. Verified against HEAD `acc3162d` (binary `jit 0.2.1`,
commit `acc3162d`, matches `git rev-parse HEAD`).

## Mechanical bar (initial audit, all clean)

- **M2 links/anchors** — `OK: all links and anchors resolve`.
- **M3 citations** — `OK: all cited paths and @/ items resolve` (incl. the new
  `.github/workflows/ci.yml` cites).
- **M5 projections** — `OK: projections fresh` (no hand-edit of projected regions).
- **M1 flag inventory** — invented-surface guard residue is all benign: non-jit tool flags
  (`--path`, `--release`, `--workspace`, `--proto`, `--tlsv1`, `--rm`, `--name`, `--entrypoint`)
  and the global jit flags `--schema`/`--version` (absent from `--schema`'s `.flags[].name`).
  No invented jit flag. Every documented jit command/flag verified present in `jit --schema`
  or `crates/jit/src/cli.rs` (`-q` is the global `quiet` flag, `cli.rs:28`).
- **M4 box-drawing** — one hit, `mcp-server/README.md:143-150`, a directory-tree/verbatim
  listing (known-benign, kept). No diagram-shaped ASCII/arrow art anywhere; every `-->` line
  in the footprint sits inside a `mermaid` fence (README stateDiagram, web + mcp flowcharts).
  REQ-03/REQ-04 satisfied without rewrite.

## Node-floor sweep (REQ-07) — 4 instances, all unified on Node 20

CI floor is Node 20 (`.github/workflows/ci.yml:204,240`, `node-version: '20'`).

| Location | Before | After |
|---|---|---|
| `INSTALL.md:160` | `Node.js 20+ (for MCP server and Web UI)` | `Node.js 20+ (for MCP server and Web UI; the CI floor, ` + "`.github/workflows/ci.yml`" + `)` |
| `INSTALL.md:261` | `**Node.js** (v18+): Required …` | `**Node.js** (20+): Required … ; the CI floor is Node 20 (` + "`.github/workflows/ci.yml`" + `)` |
| `mcp-server/README.md:322` | `Requires Node.js v18+ (the repository builds on Node.js 20):` | `Requires Node.js 20+, matching the CI floor (` + "`.github/workflows/ci.yml`" + `):` |
| `mcp-server/README.md:324` | `node --version  # Should be v18 or later` | `node --version  # Should be v20 or later` |

Every Node-floor statement now reads 20 and cites `.github/workflows/ci.yml`.

## Other drift fixed

- **jit-server default port (REQ-04, factual claim).** `README.md:55` stated the `jit-server`
  Web UI is at `http://localhost:8080`. The `jit-server` binary binds `0.0.0.0:3000` by default
  (`crates/server/src/main.rs:38`), consistent with `docs/how-to/deployment.md` (3000 throughout).
  `8080` is only the Docker nginx web container (`INSTALL.md:79`, correct there). Fixed to
  `http://localhost:3000`. Class sweep of every port/URL claim in the footprint found this as the
  sole defect; the Docker `8080`/API `3000` and Vite `5173` references are all correct.

## Rework (attempt 1) — 4 doc-review findings fixed

- **F1 [high] component-image `:latest` tags (REQ-04).** `.github/workflows/docker.yml:72-79`
  publishes the `-api`/`-web`/`-cli` component images only under `type=ref` (branch tags `main`,
  `develop`), `type=semver`, and `type=sha` — never `latest`. Only the all-in-one image gets
  `type=raw,value=latest` (`docker.yml:138`). Class sweep found **6** component `:latest` refs, all
  in `INSTALL.md`, all fixed to `:main` (the rolling default-branch tag, always published, copy-
  pasteable): the pull block (`:94,95,96`) plus the `docker run` examples for API (`:109`), Web
  (`:118`), and CLI (`:128,135`). The pull-block comment now states the tag policy. The all-in-one
  `:latest` refs (`INSTALL.md:93,148,337`) are valid and unchanged.
- **F2 [high] `jit-mcp-server --version` is not a real command (REQ-01).** `mcp-server/index.js`
  parses no argv/`--version`; the bin runs `main()`, which starts the MCP stdio server and prints a
  `Version: …` banner to stderr (`index.js:332`) — so `jit-mcp-server --version` launches the
  blocking server, it does not print-and-exit. `INSTALL.md:192` fixed to
  `which jit-mcp-server   # confirm the linked bin is on PATH` (works after `npm link`). Class
  sweep for non-working verification commands: this was the only one — `jit --version`,
  `jit version` (`Version: 0.2.1`), `jit-server --version` (clap `version`, `main.rs:28`),
  `node --version`, `rg --version` all work.
- **F3 [high] `validation.strictness` shown as active (REQ-05/behavior).** The field is inert —
  parsed but no behavioral effect (`crates/jit/src/config.rs:290-298`). It is no longer shown in
  the root README's adopter configuration example. No roadmap narration added.
- **F4 [medium] unsourced `Rust 1.80+` MSRV (REQ-06).** No `rust-version` in any manifest; CI uses
  `dtolnay/rust-toolchain@stable` (`.github/workflows/ci.yml:44`). `INSTALL.md:159` now states a
  recent stable toolchain with the CI cite and notes the workspace declares no minimum, instead of
  asserting `1.80`.

## Missing-projection-surface facts (REQ-06) — cited to source, recorded for Group-C follow-up

No projection surface exists for these; each is stated near its source and cited here.

- **No declared MSRV (engine/packaging gap).** No `rust-version` in `Cargo.toml`/crate manifests;
  CI installs `dtolnay/rust-toolchain@stable` (`.github/workflows/ci.yml:44`). The former
  `INSTALL.md:159` `Rust 1.80+` figure was unsourced; the doc now states "a recent stable
  toolchain" with the CI cite (F4). Follow-up candidate: decide whether `Cargo.toml` should declare
  a `rust-version` (MSRV) so the install floor has an authoritative source.
- **`validation.strictness` parsed-but-unused (engine gap).** The `[validation] strictness` config
  key is deserialized but has no behavioral effect (`crates/jit/src/config.rs:290-298` — "inert").
  The root README does not advertise it as an adopter setting. Follow-up candidate: either wire
  strictness to real behavior or remove the compatibility key.
- **jit-server / jit serve default ports.** `README.md:55` (3000) and the `3000–3099` auto-select
  range (`crates/jit/src/commands/serve.rs:5`, `cli.rs:418`) are hand-copied from
  `crates/server/src/main.rs:38` and `serve.rs`. No projection surface for default ports.
- **MCP operational limits.** `mcp-server/README.md:17` (`30s` timeout, `10` concurrent) copy
  `mcp-server/lib/cli-executor.js:14` (`DEFAULT_TIMEOUT = 30000`) and
  `mcp-server/lib/concurrency.js:15` (`maxConcurrent = 10`). Accurate; no projection surface.
  Note: a longer `LONG_TIMEOUT = 600000` (10 min) applies to external-checker commands
  (`cli-executor.js:17`); the README's `30s` is the default and reads as such.

## Judgment calls

- `docs/README.md` (4-line stub → `docs/index.md`): kept — valid, resolves, not obsolete.
- `README.md:224-234` config example carries `bug = 4` in `[type_hierarchy]`. Framed under
  "JIT is configurable via `.jit/config.toml`" as an editable example of a custom hierarchy,
  not as a `jit init` shipped default (the canonical `milestone → epic → story → task` is named
  at `README.md:219`). Signal present; left as-is.
- No deletion/merge performed; no inbound-link repointing needed. Footprint self-contained.

## Authorized final rework — 3 remaining doc-review findings fixed

The final rework's scoped M2 and M3 checks pass. The required M5 check initially
revealed a stale configured `CLAUDE.md` projection outside this task's footprint;
the lead corrected the projection target and the final M5 rerun passes. The
check's temporary in-place `CLAUDE.md` change was restored and is not part of
this task's diff.

- **F1 [high] inert `validation.strictness` choice list.** Removed the entire `[validation]`
  stanza from the root README's configuration example. A scoped sweep found no other
  `strictness`, `strict`, `loose`, or `permissive` configuration-choice wording in the footprint.
- **F2 [high] MCP success-response envelope.** `mcp-server/README.md` now describes the two
  `formatSuccessResult` modes in `mcp-server/index.js:139-158`: the default single text
  `content` block containing summary plus compacted JSON, and structured mode's summary
  `content` plus `structuredContent`. It reserves the `success: false` JSON object for the
  handler-generated error text and identifies `isError: true` as the MCP error signal
  (`index.js:221-322`). No success-envelope claim remains in the scoped MCP README.
- **F3 [medium] unsupported platform and resource requirements.** Removed the distribution
  floors, architecture assertion, RAM/disk figures, storage density, and SSD recommendation from
  `INSTALL.md`. The former system-requirements table of contents entry is now Optional
  Dependencies; the remaining dependency statements are tied to their documented functions, and
  the Node floor cites CI. No unsupported hardware, storage, or OS requirement remains in the
  footprint.

### Deferred-item categorization for Group C (REQ-06 handoff)

| Item | Classification | Action |
|---|---|---|
| No declared Rust MSRV | Valid missing-projection/source-of-truth gap | File a Group-C follow-up to decide and declare the supported Rust version. |
| Inert `validation.strictness` compatibility key | Valid engine/documentation-source gap | File a Group-C follow-up to remove it or give it behavior; it is not an adopter configuration setting. |
| `jit-server` / `jit serve` default ports | Valid missing-projection fact | File a Group-C follow-up only if the project wants a projection; current docs cite source. |
| MCP default timeout and concurrency limit | Valid missing-projection fact | File a Group-C follow-up only if the project wants a projection; current docs cite source. |
| Former system-resource figures | Not a deferred factual gap | Removed as unsupported; do not file a follow-up unless a reproducible benchmark is deliberately added. |

## Post-round-4 resolution — both recorded doc-review runs

The first requested result path has a transposed UUID segment; the retained record is
`.jit/gate-runs/97b1a203-c4be-401a-b755-7e403b6d23c0/result.json`. The second record is
`.jit/gate-runs/d525b0c2-75f4-4088-890e-02862407b372/result.json`. The duplicate findings
below remain individual resolution rows so both reviews are accounted for.

| Review finding | Resolution | Source-backed evidence |
|---|---|---|
| `97…/F1` — root `jit-server` overstatement | `README.md` now calls it the REST API server and limits UI serving to embedded assets or `--web-dir`. | `crates/server/src/main.rs` chooses `--web-dir`, then embedded assets, then API-only. |
| `97…/F2` — invalid archive example | Replaced the unmanaged path and output-directory value with generic `<managed-document>` and `<configured-category>` placeholders. | `jit doc archive --help` requires `PATH` and a configured `--type`; `.jit/config.toml` maps category keys such as `design`. |
| `97…/F3` — future-facing status prose | Replaced the release-series/status paragraph with present format-compatibility behavior and `jit version`. | `crates/jit/src/storage/json.rs` stores `index.json` format versions and rejects newer ones. |
| `97…/F4` — every-command MCP claim | States schema leaf-command generation and that the advertised list is a subset of generated tools. | `mcp-server/lib/tool-generator.js` recurses through `subcommands` and creates tools only in the leaf branch. |
| `97…/F5` — permanent synchronization claim | States schema loading happens once at startup and requires restart after a CLI update. | `mcp-server/lib/schema-loader.js` invokes `jit --schema`; `mcp-server/index.js` caches generated tools. |
| `97…/F6` — shell execution claim | States direct Node `execFile` launch and inherited MCP-host environment. | `mcp-server/lib/schema-loader.js` and `lib/cli-executor.js` use `execFileAsync('jit', ...)`. |
| `97…/F7` — wrong MCP version source | Documents the version loaded from `jit --schema`, not `package.json`. | `mcp-server/index.js` constructs and logs with `jitSchema.version`. |
| `d525…/F1` — MCP test PATH | The source-build recipe exports `target/release` before `npm test`. | `mcp-server/test-unit.js` invokes `jit --schema`. |
| `d525…/F2` — MCP startup prerequisite | Installation now requires built or installed `jit` on `PATH` before `node index.js`. | `mcp-server/lib/schema-loader.js` fails when `jit --schema` cannot be run. |
| `d525…/F3` — obsolete Copilot configuration | Uses `copilot mcp add`, `copilot mcp list`, and names `~/.copilot/mcp-config.json` with an official GitHub Docs link. | GitHub’s current Copilot CLI MCP guide (checked 2026-07-11) documents that command workflow and path. |
| `d525…/F4` — wrong MCP version source | Same resolved version statement as `97…/F7`. | `mcp-server/index.js` uses `jitSchema.version`. |
| `d525…/F5` — static/Vite web serving | Replaces arbitrary static/Vite serving advice with `jit-server --web-dir web/dist`; Vite is explicitly API-unproxied. | `web/src/api/client.ts` calls same-origin `/api`; `web/vite.config.ts` has no proxy; `crates/server/src/main.rs` serves `--web-dir`. |
| `d525…/F6` — Web README local workflow | Uses the same-origin built-assets workflow and says a reverse proxy is needed for separate Vite/API origins. | `web/src/api/client.ts`, `web/vite.config.ts`, and `crates/server/src/main.rs`. |
| `d525…/F7` — root `jit-server` overstatement | Same resolved qualification as `97…/F1`. | `crates/server/src/main.rs`. |
| `d525…/F8` — Web container API hostname | Individual-container instructions create a user-defined network and give the API the `api` alias. | `docker/nginx.conf` proxies `/api/` to `api:3000`; Docker’s network alias supplies that name. |
| `d525…/F9` — CLI interactive shell | Adds `--entrypoint sh` before the CLI image name. | `docker/Dockerfile.cli` sets `ENTRYPOINT ["jit"]`. |
| `d525…/F10` — all-in-one API hostname | Adds `--add-host=api=127.0.0.1` and explains the mapping. | `docker/nginx.conf` uses `api:3000`; `docker/entrypoint.sh` starts local `jit-server`; Docker documents `--add-host` as a custom host-to-IP mapping. |

### Deferred-item recheck

- `dev/active/2d109173-plan.md` delegates engine and missing-projection gaps to its Group-C
  follow-up task. Those are legitimate project-plan non-goals for this root/component
  documentation footprint.
- The MSRV, inert `validation.strictness`, default-port, and MCP-limit entries above remain
  valid missing-projection/source-of-truth follow-ups; they do not block these documentation
  corrections. The removed system-resource figures remain deliberately removed rather than
  deferred.

## Manual takeover — final round-five corrections

- **Named-volume initialization.** The individual API-container recipe now initializes
  `jit-data` through the CLI image with `/data` as its working directory before the API server
  mounts it. `jit init` initializes its current directory, and `jit-server` requires the mounted
  data directory to contain an initialized repository.
- **All-in-one UI claim.** Removed the all-in-one pull and run recipe rather than documenting an
  API-and-UI workflow that its Dockerfile and nginx root do not deliver. The Compose and individual
  component workflows remain the source-backed container paths.
- **Gate stages.** The README now names `jit issue claim` as the Ready-to-InProgress precheck
  path and retains postchecks as the completion requirement; it no longer overstates prechecks as
  a prerequisite of every way work may begin.
