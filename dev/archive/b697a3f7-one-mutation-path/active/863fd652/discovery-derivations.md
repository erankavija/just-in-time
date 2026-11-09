# Discovery derivations of `grow_discovering` callbacks

Enumeration for `863fd652` REQ-03 at commit `3fd55b8e9` (merge `d925018b7`). For each `grow_discovering` callback in `crates/jit/src`, every value it derives per round and how that value reaches its consumer.

| Callback | Value derived per round | Status |
|---|---|---|
| `commands/mod.rs:3215` (automatic precheck preflight) | issue documents | Threaded into its consumer: `DiscoveredDocuments` → `captured_gate_evidence_sharing` |
| | dependency, prompt and gate-run paths; repository history | Not consumed after discovery (capture spec only) |
| `commands/gate_check.rs:1145` (bound evaluation inputs) | issue documents | Threaded into its consumer: `BoundGateEvaluation.documents` → `captured_gate_evidence` (including the final bound image) |
| | dependency, prompt and gate-run paths; repository history | Not consumed after discovery |
| `commands/gate_check.rs:1100` (bound evaluation declarations) | rule schema paths (`rule_schema_paths`) | Not consumed after discovery; the declaration memo serves the rules parse and its schema-request outcome |
| `commands/validate.rs:860` (`capture_proposed_base_inner`) | gate registry | Threaded into its consumer: `CapturedBase` → seeded declarations (`6b22ce6d`) |
| | planned issues' documents | Threaded into its consumer: `CapturedBase` → `MutationBase::documents()` → `EvidenceComparison` → `record_captured_prechecks` |
| | strict-parsed issue records | Threaded into its consumer: the returned `issues` binding |
| | configuration | Threaded into its consumer: `ConfigurationParse.kept`; `into_config` reads the declaration memo |
| | overlaid issue ids, rules bytes, schema requests, closure paths (`validate_capture_closure`, `discover_issue_closure`) | Not consumed after discovery |
| `commands/archive.rs:433` (archive candidates) | directory listings and paths | Not consumed after discovery |
| `commands/validate.rs:2592` | none (test callback `|_, spec|`) | Not applicable |

`commands/mod.rs:384` is the `capture_discovering` wrapper that forwards its callback to `grow_discovering`; it derives nothing itself.

`discover_issue_documents` (`commands/document.rs:1546`) has one caller, `discover_gate_inputs`, and every document derivation goes through `derive_issue_documents` (`commands/document.rs:1525`), where the test counter `Resolution::Documents` sits. `test_gate_evaluation_resolves_its_inputs_once_per_captured_image` counts one document derivation per image for a manual exec evaluation and an exec precheck at HEAD and two on the parent.
