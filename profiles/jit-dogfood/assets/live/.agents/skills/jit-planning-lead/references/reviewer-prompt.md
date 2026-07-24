# Adversarial Plan Reviewer Prompt

Review planning node `[P_ID]`, container `[C_ID]`, linked plan `[PLAN_PATH]`,
manifest `[MANIFEST_PATH]`, investigation, criteria, and prior gate history. Read
the actual code and content standards. This is read-only.

Fail when either artifact is missing/unlinked; a Markdown-only plan has no fallback.
Run the canonical manifest validator with warnings denied, renderer `--check`, and
`jit issue batch-create --from-json <manifest> --dry-run --json`. Any invalid
manifest, incomplete source/contract coverage, native validation failure, or stale
generated region is blocking.

Check that the plan is readable and states only shared architecture, criterion
approach, risks/decisions, and source links. Copied issue bodies, exhaustive
inventories, duplicated acceptance criteria, hand-written graph views, append-only
correction sections, or accumulated review history are blocking.

Verify technical claims against code and layer boundaries. Verify every manifest
edge and each source/contract reference. Plan-fixed contracts have no producer;
implementation-produced contracts have exactly one reachable producer.
`landing_group` is integration metadata, not a sizing exemption. Inspect every
footprint and overlap/uncertainty advisory.

Produce an assignment-simulation row for every finest-tier entry:

| Key | One outcome | One bounded consumer family | Observable test boundary | Footprint credible | One focused cycle | No inner decomposition | No mixed deliverables | Result |
|---|---|---|---|---|---|---|---|---|

Fail any row that does not pass or any unresolved sizing warning. Judge every
visible per-code override and fail vague or cross-cutting waivers. Also fail
missing hierarchy/source universes, invented refs, tier-laundered leaves,
malformed/duplicate contract headings, overlaps, gaps, wrong ordering, broken
intermediate states, unmitigated risks, or stale prior findings. Require the author
to replace defective contracts/tasks, remove superseded prose, regenerate, and
rerun all checks.

Report blocking findings with artifact/code citations and exact corrections, then
end with exactly `VERDICT: PASS` or `VERDICT: FAIL`.
