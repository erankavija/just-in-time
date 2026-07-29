# Policy-grounded code-review live verification

**Story:** `b4e55aa2`

**Reviewed issue:** `8b479a72-237c-4113-a868-17355cef55bc`

**Gate run:** `b02b670c-ce37-4564-9f78-ac6d3a7f1099`

**Result:** passed

**Run commit:** `c3eca0a69e667b59daa3d5b0f5b87fe533d826ca`

**Reviewer session:** `019f5815-5fe0-7f71-b23a-75d16f4213f1`
**Reviewer:** OpenAI Codex CLI 0.144.1, model `gpt-5.6-terra`, reasoning effort `high`

## Purpose

This report records one representative positive execution of the repository's
policy-grounded `code-review` gate. The run reviewed the completed atomic
cutover in issue `8b479a72` and passed with no findings. It demonstrates the
integrated positive path; deterministic tests remain the evidence for malformed,
missing, contradictory, and failing inputs.

The durable gate record is
`.jit/gate-runs/b02b670c-ce37-4564-9f78-ac6d3a7f1099/result.json`. Its timestamps
are `2026-07-12T20:47:03.429392166Z` through
`2026-07-12T20:49:43.627309038Z`, with a recorded duration of 160,197 ms and
exit code 0.

## Attribution

The reviewer attributed exactly these reachable `jit:8b479a72` commits:

| Commit | Subject |
|---|---|
| `b62bf8d2161c0f0b47e054c594d967dc6a9fb918` | `feat: ground code review in canonical policy (jit:8b479a72)` |
| `ed3aff7da5e452b1a88150f67380d10a1d5afbf5` | `docs: correct review transport ownership (jit:8b479a72)` |
| `c3eca0a69e667b59daa3d5b0f5b87fe533d826ca` | `fix: keep peer review status non-blocking (jit:8b479a72)` |

The gate ran on the last commit in that set. The third commit is material to the
verified behavior: an incomplete independent review or judgment gate is
reported as current evidence, but its status alone is not an implementation
defect and does not force this code review to fail.

## Prompt, context, and session measurements

The passing result suppresses reviewer stderr by design, so its stored `stderr`
is the empty string. Consequently, the durable result alone does not retain the
usual Codex stderr header. The session identity and runtime configuration below
come from the local Codex transcript whose start timestamp, working directory,
git commit, and final response match the gate record:

`$CODEX_HOME/sessions/2026/07/12/rollout-2026-07-12T23-47-03-019f5815-5fe0-7f71-b23a-75d16f4213f1.jsonl`

Its `session_meta` records session
`019f5815-5fe0-7f71-b23a-75d16f4213f1`, repository commit
`c3eca0a69e667b59daa3d5b0f5b87fe533d826ca`, Codex CLI 0.144.1, and provider
OpenAI. Its `turn_context` records model `gpt-5.6-terra`, effort `high`, and a
read-only sandbox. The committed gate configuration independently specifies
`codex exec --sandbox read-only -m gpt-5.6-terra` with
`model_reasoning_effort="high"`.

Measurements were reproduced from the transcript's reviewer `input_text`. A
`jq` capture split it at the wrapper's `## Context` JSON fence and measured
Unicode characters with `length`, UTF-8 bytes with `utf8bytelength`, and lines
by splitting on newline:

| Input component | Characters | UTF-8 bytes | Lines | Evidence |
|---|---:|---:|---:|---|
| Committed `scripts/code-review-prompt.md` | 7,299 | 7,301 | 72 | `wc -m -c -l` at `c3eca0a6`, including its final newline |
| Prompt segment transmitted by the wrapper | 7,298 | 7,300 | 72 | Transcript capture; shell command substitution removes the file's trailing newline |
| Compact gate context JSON | 6,307 | 6,307 | 1 | Transcript capture; parsed successfully as one JSON object |
| Complete wrapper-composed reviewer input | 15,230 | 15,234 | 98 | Transcript's reviewer `input_text`, including the common findings/verdict contract |

These are direct measurements, not token estimates. The transcript separately
records cumulative session accounting of 943,881 input tokens, including
797,184 cached input tokens, 7,316 output tokens, and 4,225 reasoning-output
tokens (951,197 total). Those cumulative values include repeated context across
the review's tool turns; they are not presented as the token size of the initial
prompt or gate context. The same final event's last-call counters are 82,893
input tokens, including 80,640 cached input tokens, 744 output tokens, and 516
reasoning-output tokens (83,637 total). They describe only the final model call,
not the whole review or the initial input. The recorded model context window is
353,400 tokens.

The compact context carried the issue description, labels, dependencies,
documents, the four current gate projections, and one latest prior structured
code-review run. At review time the projections were `cargo-ci`,
`repo-validate`, `doc-review`, and `docs-mechanical`, each passed with exit code
0. Consuming this evidence means reporting and interpreting it for its purpose;
it does not impose a blanket requirement that every independent review be
complete before code review can make its own judgment.

## Applicable canonical prose

The attributable paths were under `scripts/`, `contrib/`, `crates/`, `docs/`,
and `dev/`. Repository discovery found one applicable instruction file and no
nested instruction file in any affected subtree:

- `AGENTS.md` — applicable from the repository root to every affected path.

Therefore `AGENTS.md` was the complete prose baseline for this run. The result's
`Policy sources` field additionally names configuration and registry sources
used to resolve addressable policy; those files supplement the prose baseline
without becoming additional `AGENTS.md` files.

## Resolved addressable policy

The reviewer emitted six resolved qualified IDs. Re-running `jit item show` for
each ID succeeds. Their configured ownership, read from `.jit/config.toml`, is:

| Qualified ID | Configured source of truth | Source used |
|---|---|---|
| `@/charter/D-2` | markdown-first | `dev/vision/9db27a3a-charter.md` |
| `@/charter/D-6` | markdown-first | `dev/vision/9db27a3a-charter.md` |
| `@/invariant/atomic-writes` | registry-first | `.jit/invariants.toml`, `invariants` table |
| `@/invariant/pid-safety` | registry-first | `.jit/invariants.toml`, `invariants` table |
| `@/invariant/single-source-prose` | registry-first | `.jit/invariants.toml`, `invariants` table |
| `@/gate/jit-validate` | registry-first | `.jit/gates.toml`, `gates` table |

The invariant projection in `AGENTS.md` and the gate projection in
`docs/reference/rules-and-gates.md` are renderer-owned views. The run used the
configured registry or Markdown source above as authoritative, consistent with
`@/charter/D-6`.

## Exact emitted evidence header

The stored stdout begins with exactly these five fields:

```text
Attribution: b62bf8d2, ed3aff7d, c3eca0a6 (`jit:8b479a72`)
Policy sources: AGENTS.md; .jit/config.toml; .jit/invariants.toml; .jit/gates.toml; dev/vision/9db27a3a-charter.md
Resolved items: @/charter/D-2, @/charter/D-6, @/invariant/atomic-writes, @/invariant/pid-safety, @/invariant/single-source-prose, @/gate/jit-validate
Gate evidence: cargo-ci passed (0); repo-validate passed (0); doc-review passed (0); docs-mechanical passed (0)
Truncation recovery: none
```

`Truncation recovery: none` is the complete truncation record for this positive
run. The reviewer used bounded reads partitioned by tagged commit, changed path,
and relevant line range; it did not report any relevant truncated evidence that
required a narrower recovery read.

## Verdict and findings contract

The human-readable enumeration recorded `Total findings: 0`. The exact parsed
payload persisted in `result.json` is:

```json
{
  "verdict": "pass",
  "summary": "All attributable policy-discovery, wrapper, regression-test, and documentation changes satisfy the issue criteria.",
  "findings": []
}
```

There are zero parsed findings, so the complete collection of per-finding
`references` arrays is also empty: `[]`. No finding or references array has been
omitted from this report.

The stored stdout contains both line-exact findings fence markers, the one-line
JSON object, `VERDICT: PASS`, and the wrapper's `Gate result: PASSED`. The parsed
verdict is `pass`, the process exit code is 0, and the gate status is `passed`;
the human, structured, terminal, and process-level representations agree.

## What the live run demonstrates

- **Bounded inspection:** the transcript partitions commit attribution and
  patches, then narrows current-source checks to affected paths and relevant
  line ranges. Its final check confirms only the root `AGENTS.md` applies.
- **Current gate evidence:** the transmitted context contains the latest status
  and exit code for each required gate; the evidence header reproduces all four.
  The reviewer does not rerun passing gates and does not treat peer-review
  completion as a categorical prerequisite.
- **Qualified-item resolution:** every emitted ID resolves, and the report above
  records the configured Markdown-first or registry-first source actually used.
- **Terminal contract:** the numbered count, structured payload, terminal
  verdict, process exit code, and stored status form one consistent passing
  result with no findings.

## Deterministic failure-path evidence

This report deliberately does not add a standalone live negative fixture.
Failure behavior is reproducible in deterministic tests, including:

- `crates/jit/tests/code_review_policy_test.rs` checks bounded discovery,
  root-to-path policy precedence, source resolution, relationship claims, all
  five evidence fields, use of current gate evidence, the non-blocking treatment
  of incomplete peer review, and absence of copied registry prose or a duplicated
  wrapper contract.
- `crates/jit/tests/ai_review_verdict_tests.rs` checks failing and unparseable
  verdicts, advisory passing findings, classified referenced findings, legacy
  findings without `references`, wrapper parity, and safe prompt transport.
- `crates/jit/src/domain/gate_findings.rs` checks malformed or incomplete
  findings fences, last-complete-block selection, missing and multiple
  references, and rejection of non-string reference members.
- `crates/jit/src/storage/gate_runs.rs` checks round trips and legacy stored
  findings whose missing `references` field defaults to an empty collection.

These tests make negative paths stable and repeatable while the representative
live evidence remains a positive integration record.
