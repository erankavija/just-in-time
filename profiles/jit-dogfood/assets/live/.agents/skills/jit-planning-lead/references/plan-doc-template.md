# Plan Document Template

```markdown
# Plan: <Container Title> (<C-short-id>)

> Planning node: <P-short-id>. Authoritative graph:
> [breakdown.json](breakdown.json).

## Outcome and criterion approach

| Criterion | Approach | Evidence / open gap |
|---|---|---|
| REQ-01 | <design approach, not copied criterion/task prose> | <source citation> |

## Shared architectural contracts

### `<semantic-contract-id>` [plan-fixed] — <name>

<One definition consumed through `contract_refs`; implementation-produced
contracts use `[implementation-produced]` instead and name one producer through
`produces_contracts`. Cite code or investigation.>

## Generated decomposition overview

<!-- jit:breakdown-overview:begin -->
<!-- jit:breakdown-overview:end -->

## Material risks and owner decisions

| Risk / decision | Resolution and rationale |
|---|---|
| ... | Chosen ...; rejected ... because ... |

## Investigation sources

- [Investigation](<linked-path>) — exhaustive consumers/files remain there.
```

The manifest is the sole authority for issue bodies, task criteria, keys, and
edges. The plan states each shared fact once and contains no copied descriptions,
consumer inventories, acceptance criteria, review history, or hand-written task
table/DAG. The helper owns the marked region.
