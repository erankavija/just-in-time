# Remaining-artifact addendum — development root

The completeness record at `dev/active/8e071e18-dev-artifact-layout/archive-completeness-record.md`
enumerates the managed development areas at the commit that added it. Records written after that
commit changed the population, so this addendum carries the enumeration forward to the state
`143388dc` closes on and states the reason each added file remains. Issue `143388dc`, REQ-03.

## Method

The enumeration in the completeness record is 138 paths, parsed from its per-area tables. The
current population is `git ls-files` over the eight areas `[documentation].managed_paths` declares.
The two sets are compared directly, so the delta is measured rather than recalled. Owners come from
the `documents` entries of every record in `.jit/issues/`, and an owner's position is its place in
the transitive dependency closure of the 24 archived containers.

## Delta

| population | count |
|---|---|
| enumerated by the completeness record | 138 |
| present now | 145 |
| added since | 8 |
| removed since | 1 |

Every added file is a record this epic wrote after the completeness record. Seven sit in the epic's
own directory and the eighth is this addendum, which counts itself: writing it into a managed area
is what makes it part of the population it enumerates. The single removal is an artifact a re-run
relocated.

### Added

| path | reason it remains | evidence |
|---|---|---|
| `dev/active/8e071e18-dev-artifact-layout/disposition-design-documents.md` | owner outside every archived subtree | referenced by `93aaa2b2` (done), inside no archived container's hierarchy |
| `dev/active/8e071e18-dev-artifact-layout/disposition-implementation-plans.md` | owner outside every archived subtree | referenced by `079ea42e` (done), inside no archived container's hierarchy |
| `dev/active/8e071e18-dev-artifact-layout/disposition-reference-material.md` | owner outside every archived subtree | referenced by `2d7ae27e` (done), inside no archived container's hierarchy |
| `dev/active/8e071e18-dev-artifact-layout/disposition-session-records.md` | owner outside every archived subtree | referenced by `5a19fffb` (done), inside no archived container's hierarchy |
| `dev/active/8e071e18-dev-artifact-layout/disposition-study-records.md` | owner outside every archived subtree | referenced by `66c467c1` (done), inside no archived container's hierarchy |
| `dev/active/8e071e18-dev-artifact-layout/disposition-showcase-theme.md` | live owner | referenced by `8e071e18` (in_progress) |
| `dev/active/8e071e18-dev-artifact-layout/handoff-4.md` | live owner | referenced by `8e071e18` (in_progress) |
| `dev/active/143388dc-development-root-cleanup/remaining-artifacts-addendum.md` | live owner | this record, referenced by `143388dc` (in_progress, claimed `agent:jit-execution-lead`) |

The five `owner outside every archived subtree` rows are the same class the completeness record
already carries for `8e071e18-breakdown.json`, `8e071e18-investigation.md`, `8e071e18-plan.md` and
`ca832358/req05-archival-execution-evidence.md`: a terminal child of `8e071e18` owns them, and
`8e071e18` is the container that would relocate them, so they archive when the epic itself archives.
The record states that scope structurally — the set grows as children close — and these five are
that growth.

The last row is this addendum. A record that enumerates a population it is written into belongs to
that population, so it is counted rather than exempted.

### Removed

| path | what happened |
|---|---|
| `dev/active/planning-bracket-showcase/themes/gruvbox.css` | relocated by the re-run of container `2fbd2a82` and now at `dev/archive/2fbd2a82-planning-bracket/dev/active/planning-bracket-showcase/themes/gruvbox.css`, recorded in `dev/active/8e071e18-dev-artifact-layout/disposition-showcase-theme.md` |

Its former parent directory `dev/active/planning-bracket-showcase/themes` is now empty. Git records
no empty directory, so it is absent from a checkout built from the committed tree while the path
resolves in the repository — the same condition the completeness record describes for the seven
directories the original runs drained.

## Standing

With this addendum, every file under a managed development area carries a recorded reason it
remains: 138 in the completeness record, 8 added here, and 1 of the 138 relocated since and
recorded here as removed. That is 145, the population `git ls-files` reports over the eight areas.
