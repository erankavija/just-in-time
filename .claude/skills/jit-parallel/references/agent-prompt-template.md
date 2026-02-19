# Sub-Agent Prompt Template

Fill in the bracketed fields for each dispatched agent. Remove sections that don't apply.

---

You are [implementing / reviewing] issue [SHORT-ID] in the JIT repository at /home/vkaskivuo/Projects/just-in-time.

## Issue

**Title:** [TITLE]
**ID:** [FULL-ID]

[FULL DESCRIPTION — paste verbatim from jit issue show]

## [For implementation tasks] What to do

1. [Derive concrete steps from the issue description and acceptance criteria.]
2. Write tests first (TDD). Run `cargo test <feature> -- --nocapture` to confirm they fail, then implement.
3. Run the full suite when done:
   ```bash
   cargo test --workspace --quiet
   cargo clippy --workspace --all-targets
   cargo fmt --all
   ```

## [For review tasks] What to do

1. Locate the relevant code (search for key symbols from the issue description).
2. Verify each acceptance/success criterion is met.
3. Run `cargo test --workspace --quiet` and `cargo clippy --workspace --all-targets`.
4. If complete: pass the `code-review` gate with `mcp__jit__jit_gate_pass` (id="[SHORT-ID]", gate_key="code-review", by="agent:claude"), then set state=done with `mcp__jit__jit_issue_update`.
5. If incomplete: do NOT pass the gate. Return a detailed description of what is missing.

## Coding conventions (from CLAUDE.md)

- Functional style, no unsafe code, `Result`-based errors with `thiserror`
- Test naming: `test_<function>_<scenario>`
- Use `TestHarness` from `crates/jit/tests/harness/` for in-process command tests
- Add tests to an existing related test file where one exists
- Zero clippy warnings — fix any warnings you encounter, even pre-existing ones in files you touch

## Return

Return a summary of:
- Files modified or created
- Tests added (names)
- Output of `cargo test --test <file>` confirming they pass
- Any issues encountered
