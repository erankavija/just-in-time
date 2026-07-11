# File Conflict Heuristics

Use these rules to determine whether two issues can run in parallel safely.

## High-risk files (serialise if either issue touches these)

- `crates/jit/src/domain/types.rs` — Event enum, Issue struct; match exhaustiveness cascades everywhere
- `crates/jit/src/main.rs` — monolithic run() function; any two CLI additions will conflict
- `crates/jit/src/lib.rs` — module declarations and re-exports
- `crates/jit/src/commands/mod.rs` — CommandExecutor struct definition
- `Cargo.toml` / `crates/jit/Cargo.toml` — dependency additions conflict easily

## Safe to parallelise

- New test files (`crates/jit/tests/<new_file>.rs`) — no existing content to conflict
- Separate command files (`commands/foo.rs` vs `commands/bar.rs`) if both are new or the issue only appends an `impl` block
- Separate domain submodules (`domain/queries.rs` vs `domain/types.rs` if only types.rs adds a variant and queries.rs adds a function)
- Documentation files in `docs/` or `dev/` — markdown edits rarely conflict

## Signals that predict overlap

- Both issues mention the same command (e.g., both touch `gate check` output)
- Both issues add a new `Event` variant — forces edits to `types.rs` match arms
- Both issues add CLI flags — likely both touch `cli.rs` and `main.rs`
- Both issues add tests to the *same existing* test file — insertion-point conflict on merge

## Quick check procedure

For each pair of candidate issues:
1. Read the description and identify the primary files likely touched.
2. If any file appears in both lists and is not a new file, serialise.
3. If unsure, run a quick `grep` to find where the relevant symbols currently live before deciding.
