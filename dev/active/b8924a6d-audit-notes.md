# Audit notes — How-to guides (b8924a6d)

Footprint: `docs/how-to/` (10 files, ~4,452 lines). Verified every CLI/storage/
structure/semantics claim against HEAD (`8e4acd98`, `jit 0.2.1`). Binary confirmed
at HEAD before trusting `--schema`.

## M4 result (REQ-04 diagram-shaped art)

Zero diagram-shaped blocks. `grep -rnP '[\x{2500}-\x{257F}]' docs/how-to/` returns
only the known-benign carve-out at `dependency-management.md:139-140` and `:419` —
verbatim `jit graph deps` tree output inside a `#`-commented example block, not a
hand-drawn diagram. Kept as plain text per the doc-review carve-out. All existing
Mermaid blocks (cycle/reduction/edge figures) are already `mermaid`-fenced.

## Drift classes swept (REQ-01/REQ-04)

Three classes found; each swept across the whole footprint, not patched line-by-line.

1. **Invented `query … --filter "labels.X:Y"`** — `jit query all/available/blocked`
   have no `--filter` flag (only `--label`); `--filter` errors ("unexpected argument").
   The `labels.<ns>:<val>` dotted grammar is also wrong — the boolean filter grammar
   (used by the valid `issue update --filter`) is `label:<ns>:<val>` with `AND/OR/NOT`
   (`crates/jit/src/query_engine/mod.rs:4`). Fixed to `--label "<ns>:<val>"`
   (repeatable = AND). 24 sites: `software-development.md` (23) + `custom-gates.md:679`.
   Two of the converted `query all` pipelines jq over `.dependencies` / `.created_at`,
   which the default list envelope omits; added `--full` there
   (`software-development.md:332,476`). Left untouched: `custom-gates.md:847`
   (`jit issue update --filter "label:epic:auth"`) — `issue update` DOES take
   `--filter` and `label:` is the correct grammar (`cli.rs:930`).

2. **JSON list-envelope mishandling** — enveloped output treated as a bare array or
   wrong key. Authoritative shapes from `cli.rs`: `graph deps`→`{count,nodes}`,
   `graph rdeps`(alias `downstream`)→`{count,dependents,issue_id}` (`cli.rs:2040-2042`),
   `graph roots`→`{count,roots}` (`cli.rs:2057`), `query *`→`{count,issues}`,
   `issue show` `.dependencies[]` = objects. Fixed in `dependency-management.md`:
   `:463` `.[] | .id` → `.roots[] | .id`; `:496` `jq 'length'` → `jq '.dependents | length'`
   (matches the already-correct `software-development.md:335`); `:613`
   `.dependencies[]` → `.dependencies[].id`.

3. **`config show <key>` invalid** — `config show` takes no key argument (errors);
   the single-key getter is `config get <key>` (`cli.rs:2424`). Fixed
   `troubleshooting.md:355` (`config show worktree.mode` → `config get worktree.mode`).
   Only site; `config set <key> <value>` at `:368` is correct.

Separate single-instance fix (invalid CLI value, not a multi-site class):
`dependency-management.md:376` used `--state backlog,ready,in_progress` — `--state`
is a single `Option<String>` and rejects a comma list ("Invalid state: …"). Rewrote
to `jit query all --json | jq '… select(.state == "backlog" or …)'` preserving the
"hide done issues" intent.

## Verified-clean, no change

- `jit-server --data-dir/--bind/--web-dir` (`crates/server/src/main.rs:35,39,45`),
  `JIT_DATA_DIR`/`JIT_LOCK_TIMEOUT`/`RUST_LOG` (deployment.md).
- Full `gate`/`gate preset` surface; `jit gate preset show rust-tdd` output is
  verbatim-accurate (custom-gates.md).
- All `claim` subcommand arg/flag shapes incl. issue-id-vs-lease-id split; `worktree
  info/list`; `issue claim-next`; env overrides `JIT_WORKTREE_MODE`/`JIT_ENFORCE_LEASES`
  (`config.rs:2187,2228`); config keys under `[worktree]`/`[coordination]`
  (multi-agent-coordination.md, troubleshooting.md).
- `.git/jit/claims.jsonl` / `claims.index.json` / `locks/` paths are correct
  (leases live under `.git/jit/`, not `.jit/`) — these escape M3's prefix filter, so
  verified by hand against `storage/claim_coordinator.rs`.
- All 13 validation assertion-kind names match the serde renames in
  `validation/rules.rs:1167-1194` (validation-rules.md). The 7 state tokens listed at
  `validation-rules.md:99` match `domain/types.rs:28-42`.
- `plan` template block in adopt-planning-bracket.md matches
  `docs/examples/sdd/templates.toml`; `jit apply` args/flags; the four bracket
  checker scripts exist under `scripts/`.
- `doc add`/`doc history` arg+flag shapes (knowledge-work.md, research-projects.md).

## REQ-06 — volatile facts with no projection surface (recorded for follow-up)

None required an inline citation change; all are stable-and-correct enumerations or
cite their mechanism. Recorded judgment calls:

- **Builtin preset enumeration** (custom-gates.md:571-576) already names
  `jit gate preset list` as "the authoritative source" — mechanism cited, acceptable.
- **State-token enumeration** (validation-rules.md:99, 7 tokens) has no live
  projection surface: `jit --schema`'s `State` enum is buggy (hardcodes 6, omits
  `rejected` — `crates/jit/src/schema.rs:463-469`), so the source of truth is
  `crates/jit/src/domain/types.rs:28-42`. The list is currently correct and complete.
  The `--schema` State-enum defect is a pre-existing engine gap already tracked by the
  container's Group-C follow-up task; not re-filed here.
- **Assertion-kind table** (validation-rules.md:128-156) — grammar contract, no
  projector; matches source. Source has one additional kind (`type-hierarchy`,
  `rules.rs:1187`) the how-to omits; an omission is not a drift claim, left as-is.
- **Indefinite-lease policy limits** (multi-agent-coordination.md:133-134; also
  troubleshooting.md:155,174) — defaults 2/agent, 10/repo, marked "(configurable)",
  match `hierarchy_templates.rs:269-270`. Stable defaults; no config-default projector.

## Mechanical bar — clean

`./scripts/docs-mechanical.sh docs/how-to/` → M2/M3/M5 all OK, exit 0.
M1 residue is benign only (markdown anchor-link fragments, non-jit tool flags
cargo/git/docker/systemd, valid `jit-server` flags). M5 projected regions untouched.
