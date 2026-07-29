# Disposition — the showcase alternate theme

One unprefixed active-area artifact fell outside all three sets the active-area
disposition issues enumerate, so its disposition is recorded here.

## The artifact

`dev/active/planning-bracket-showcase/themes/gruvbox.css` — an alternate theme of the
planning-bracket showcase deck. It is not a design document (`93aaa2b2`), an
implementation plan (`079ea42e`), or reference material (`2d7ae27e`), so none of those
three sets covers it, and `@/issue/8e071e18/requirement/REQ-10` requires every
unprefixed artifact in the active area to carry one of three dispositions with none
left undecided.

## Why no run selected it

Its bundle held four files. Container `2fbd2a82`'s run relocated three: `talk.html`
as the artifact its issue links directly, and `base.css` and `themes/rust.css` as
embedded assets reached from the deck. Nothing reaches `gruvbox.css` — the deck loads
`rust.css` — and archival owns artifacts by reference rather than by filename, so
discovery never saw it.

## Disposition: archived location

`dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-showcase/themes/gruvbox.css`

Reached through the mechanism rather than by hand, per
`@/issue/8e071e18/decision/D-10`. The missing reference was added to `2fbd2a82` and
that container's archival was re-run: its `.jit-container` marker already resolved the
existing destination, so the run relocated this one artifact into the directory holding
the rest of the bundle and relinked the reference to the archived path. The plan
reported one pending deletion, the two artifacts already at their destinations
reconciled as already-archived, and whole-repository validation passes afterwards.

The archived bundle now holds both themes; before the re-run it held one.

## Empty directories under the managed areas

The re-run drained `dev/active/planning-bracket-showcase/themes` and its parent, so the
current count is **eight**, superseding the seven recorded in
`archive-completeness-record.md`:

```
dev/active/planning-bracket-showcase/themes
dev/design
dev/experiments
dev/plans
dev/presentations/1cc809de/vendor/fonts
dev/presentations/1cc809de/vendor/reveal.js/plugin/highlight
dev/presentations/cdc840ad
dev/sessions
```

Git records no empty directory, so no git listing reports these and a checkout built
from the committed tree does not reproduce them, while in the repository the paths
resolve. `REQ-01`, `REQ-03` and `REQ-10` range over artifacts, and an empty directory is
none, so no enumeration or count elsewhere changes.
