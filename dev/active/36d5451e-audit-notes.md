# Audit notes — Root and component documentation audit (36d5451e)

Footprint: `README.md`, `INSTALL.md`, `mcp-server/README.md`, `web/README.md`,
`docs/index.md`, `docs/README.md`. Verified against HEAD `acc3162d` (binary `jit 0.2.1`,
commit `acc3162d`, matches `git rev-parse HEAD`).

## Mechanical bar (self-verified, all clean)

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

## Missing-projection-surface facts (REQ-06) — cited to source, recorded for Group-C follow-up

No projection surface exists for these; each is stated near its source and cited here.

- **Rust version floor.** `INSTALL.md:159` states `Rust 1.80+`. No MSRV is declared anywhere:
  no `rust-version` in `Cargo.toml`/crate manifests, and CI installs `dtolnay/rust-toolchain@stable`
  (`.github/workflows/ci.yml:44`). The `1.80+` figure is unsourced and unverifiable against the
  tree (edition is `2021`, needs ≥1.56 only). Left as-is (no authoritative target to change it to);
  recorded for follow-up (declare an MSRV, or drop the specific figure).
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
