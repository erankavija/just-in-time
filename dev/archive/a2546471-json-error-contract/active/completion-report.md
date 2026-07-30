## Epic Complete: Structured failure reporting across the machine-readable CLI surface (a2546471)

**Started:** 2026-07-28T01:13:41+03:00  
**Completed:** 2026-07-29T01:30:16+03:00  
**Assignee:** agent:jit-execution-lead

### Summary

Every command arm that accepts machine-readable output now reports failures reached after successful parsing and dispatch through the canonical typed error envelope. The command-reflected conformance census covers 100 unique arms (98 invoked probes and 2 justified exemptions), the error code determines the process status, payload-stream purity is enforced, and adopter documentation has one canonical home.

### Metrics

| Metric | Value |
|---|---|
| Children completed | 43 / 44 issue entries; 1 explicitly rejected under authoritative plan D-5 |
| Waves executed | 12 |
| Rework cycles | 28, derived from `progress.json` `rework_counts` |
| Escalations | 4, all resolved |
| Sub-agent dispatches | At least 44 issue executions; auxiliary review dispatches were not separately persisted |
| Issues created during execution | 9 |

### Success Criteria

- [x] REQ-01 — `a1322bee`, `7491da4a`, `4437d897`, and `ab126109` derive the live arm set from command definitions and publish one typed census row per unique arm.
- [x] REQ-02 — `4087cd55`, `dbe2f2a8`, `cc2fd90d`, `b2295bbb`, and `002e9479` make post-dispatch failure rendering structural and typed, including configuration validation.
- [x] REQ-03 — `60aeaa49`, `7c50ef89`, and `4087cd55` require the payload stream to contain exactly one JSON envelope without adjacent plain text.
- [x] REQ-04 — `7e0c04d0`, `a193a7cc`, `72835d56`, `4359178b`, and `002e9479` bind every registered code to its class-correct exit status and verify invocation-form parity.
- [x] REQ-05 — `f4346059`, `37e3edf3`, `7491da4a`, and the namespace surveys drive a real post-dispatch failure for every non-exempt arm and assert structure and classification semantically.
- [x] REQ-06 — `a1322bee`, `ced03648`, and `ab126109` fail on missing, stale, duplicate, or planning-artifact-dependent registry coverage while keeping command reflection authoritative.
- [x] REQ-07 — `9eadd135`, `ec029993`, `85dbcd97`, and `d780dfc1` place the scoped post-dispatch contract in the canonical CLI command reference and project the code/status vocabulary.

### Wave Execution Log

**Wave 1:** Fifteen namespace surveys and the enumerable error-code foundation.  
**Wave 2:** Registered orphan codes and assembled the typed failure-lever registry.  
**Wave 3:** Repaired preset, server, recovery, search, stored-record, and generated-reference surfaces.  
**Wave 4:** Bound top-level rendering and plain/JSON exit-status parity.  
**Wave 5:** Closed structural-envelope, vocabulary, validation, claim, and shared-fixture gaps.  
**Wave 6:** Added canonical envelope, payload-purity, and arm-completeness suites.  
**Wave 7:** Completed per-arm conformance, the published census, and canonical adopter prose.  
**Wave 8:** Closed the documentation story and single-home review.  
**Wave 9:** Rejected parser-envelope work under authoritative D-5 and narrowed prose to post-dispatch scope.  
**Wave 10:** Converted invalid `config validate --json` results to the canonical failure envelope.  
**Wave 11:** Removed planning-artifact test dependencies and completed the typed constructor cutover.  
**Wave 12:** Enforced one failure-lever registration per reflected command path.

### Key Decisions

- Kept Clap parser diagnostics, help, and version exits outside scope exactly as authoritative plan decision D-5 requires.
- Made command reflection, rather than a committed count, authoritative for the live arm universe; the census is a generated artifact.
- Preserved existing wire code spellings while making `ErrorCode` the only production envelope classification and the sole source of exit status.
- Retained one `serve` registry lever for its one reflected arm while keeping dedicated semantic coverage for start, stop, and status branches.

### Escalations

- External code review required explicit authorization; the invoker authorized it and the affected gate passed.
- A stale config-get assertion exceeded the normal rework limit; the invoker authorized one narrow repair.
- A holistic review attempted to include pre-dispatch parser failures; the invoker reaffirmed the plan as authoritative, so parser behavior stayed unchanged and prose was corrected.
- `config validate --json` exceeded the epic rework limit; the invoker authorized the narrow envelope repair.

### Issues Discovered During Execution

- `cc2fd90d` — envelope failing validation reports.
- `4359178b` — classify underlying claim I/O failures uniformly.
- `4437d897` — publish the per-arm conformance census.
- `8f5e5921` — parser-envelope task, rejected before implementation under plan D-5.
- `d780dfc1` — scope canonical prose to post-dispatch failures.
- `b2295bbb` — envelope invalid configuration validation results.
- `ced03648` — make the registry independent of planning artifacts.
- `002e9479` — complete the typed error-envelope constructor cutover.
- `ab126109` — enforce one failure-lever registration per command path.

### Holistic Quality Notes

- The final `repo-validate` and independent `holistic-review` gates both passed at commit `95c38446` with zero holistic findings.
- The registry is archive-safe, typed, unique by reflected path, and reconciled against command definitions; its generated census remains 100 unique arms.
- All production error envelopes carry `ErrorCode`, and their process status is derived from that code, making raw-code and independent-status bypasses unrepresentable.
- The remaining membership divergence for `d780dfc1` is advisory repository metadata; the DAG is authoritative and repository validation passes.
