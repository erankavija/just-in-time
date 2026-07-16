# File Conflict Heuristics

Use these rules to decide whether two issues can run in parallel safely.

## High-risk surfaces

Serialize work when both issues are likely to touch the same:

- central command dispatcher or argument-definition file;
- shared domain type or exhaustive event/state match;
- crate/module export index;
- workspace dependency manifest or lockfile;
- existing integration-test suite with one common insertion point;
- generated reference or registry projection.

## Usually safe to parallelize

- Distinct new files with independent module wiring.
- Separate feature modules whose public contracts are already stable.
- Disjoint documentation pages.
- Tests in separate cohesive suites when neither issue changes shared fixtures.

## Signals that predict overlap

- Both issues mention the same command, output shape, or registry.
- Both add a state/event variant or public field.
- Both add CLI flags through one dispatcher.
- Both modify the same existing test file or generated document.

## Quick check

1. Read each issue and identify likely files and symbols.
2. Search the repository to verify where those symbols live.
3. Serialize any pair sharing a mutable existing file unless the edits have
   clearly disjoint regions and the merge risk is low.
4. Record the decision before dispatch so the wave plan is reproducible.
