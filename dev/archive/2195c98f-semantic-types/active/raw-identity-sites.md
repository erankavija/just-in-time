# Remaining raw-string identity sites

Inventory for task 1a3db2c3 REQ-03: production sites under `crates/` that hold issue-id, gate-key or gate-run-id text as a string, each with the boundary that allows it. Paths are relative to `crates/jit/src` unless stated.

## Unresolved input

- `cli.rs`, the parsers in `main.rs`, and the extractors in `crates/server/src/routes.rs`: argument and path text before parsing.
- `commands/gate_check.rs` label targets and `commands/search.rs` queries: user text that is not yet an identity.
- `errors.rs` `DependencyBatchRejectedError.rejected` and `storage/errors.rs` `AmbiguousIdError`: request text and rendered candidates reported back as given. Unresolved text is not an `IssueId`.
- `storage/errors.rs` `IssueNotFoundError::id`: a display accessor over the error's text.

## Parsing and serialization

- `domain/identity.rs`: the newtype's representation, serde and normalized prefixes.
- `storage/json.rs`, `storage/memory.rs`, `repository_state/mutation.rs`, `commands/mod.rs`: record file names parsed into typed ids.
- `gate_execution.rs`: environment serialization for a gate's child process.
- `test_cli.rs`: the CLI serialization boundary of the test helper.

## Rendering

- `commands/graph.rs` `BoundaryEdge`, `output.rs`, `visualization.rs`, and the `short_id` fields in `crates/server/src/routes.rs`: display text, including short ids derived by `IssueId::short_id()`.
- `domain/artifact_conformance.rs`: file-name and report prefixes.
- `commands/template_expand.rs` and `repository_state/rules_gates_projection.rs`: interpolation and prose rendering.

## Not identities

- Graph test fixtures use symbolic ids through `GraphNode`'s associated id type.
