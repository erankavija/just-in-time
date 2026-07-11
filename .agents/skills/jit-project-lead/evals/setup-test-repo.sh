#!/usr/bin/env bash
# Creates an isolated test repo for jit-project-lead evals.
# Usage: ./setup-test-repo.sh <target-dir> <scenario>
# Scenarios: "two-tier" | "collapsed" | "fallback" | "routing"
#
# The skeleton scenarios (two-tier | collapsed | fallback) exercise pre-flight +
# tier derivation + the front-door stop-and-ask (a generic prompt has no explicit
# mode signal, so the front door asks which mode); they do not mutate .jit/ issue
# state. So each repo only needs a valid .jit/ (config + optional templates) and
# one git commit so `jit recover` (pre-flight step 2) succeeds. The canonical
# content-standards doc the skill reads is resolved from the jit repo through the
# ~/.agents/skills symlink, not from this test repo.
#
# The `routing` scenario adds the this-repo two-tier ruleset AND one existing
# anchor-tier (type:milestone) strategic container with two sub-strategic
# (type:epic) children carrying the milestone's membership label, so the four
# mode-routing evals have a real container for mode 1 to resolve. It prints
# `MILESTONE_ID=<short_id>` on the last line for the runner to substitute into the
# mode-1 prompt's {CONTAINER_ID} placeholder. Modes 2/3/4 ignore the container.
set -euo pipefail

JIT=/home/vkaskivuo/.cargo/bin/jit
SRC=/home/vkaskivuo/Projects/just-in-time      # source of the in-repo rulesets
TARGET="${1:?Usage: setup-test-repo.sh <target-dir> <scenario>}"
SCENARIO="${2:?Scenario required: two-tier | collapsed | fallback | routing}"

rm -rf "$TARGET"
mkdir -p "$TARGET"
cd "$TARGET"

git init -q -b main
git config user.name "Test User"
git config user.email "test@example.com"

$JIT init >/dev/null

case "$SCENARIO" in

two-tier)
  # This repository's own ruleset: strategic_types = [milestone, epic];
  # the `plan` template applies_to = [epic]. Derivation must yield
  # anchor=milestone, boundary={epic}, shape=two tier.
  cp "$SRC/.jit/config.toml" .jit/config.toml
  cp "$SRC/.jit/templates.toml" .jit/templates.toml
  ;;

collapsed)
  # The research example ruleset: strategic_types = [goal]; the `plan`
  # template applies_to = [goal]. Derivation must yield anchor=goal,
  # boundary={goal}, shape=collapsed single tier.
  cp "$SRC/docs/examples/research/config.toml" .jit/config.toml
  cp "$SRC/docs/examples/research/templates.toml" .jit/templates.toml
  ;;

fallback)
  # Bare `jit init`: strategic_types = [milestone, epic] present, but NO
  # .jit/templates.toml (the BOUNDARY SET has no source). Derivation must
  # route to the numeric-level fallback, recover a proposal
  # (anchor=milestone, boundary={epic}), and STOP for confirmation.
  rm -f .jit/templates.toml
  ;;

routing)
  # This-repo two-tier ruleset (reuses the `two-tier` ruleset-copy logic) PLUS
  # one existing anchor-tier (type:milestone) strategic container with two
  # sub-strategic (type:epic) children carrying the milestone's membership label
  # (milestone:demo-release). This is a minimal-but-valid strategic container:
  # the milestone depends on both epics (milestone -> epic containment edges), so
  # the tree is connected and `jit validate` passes. Mode 1 resolves this
  # container by id; modes 2/3/4 do not need it but run fine against the seed.
  cp "$SRC/.jit/config.toml" .jit/config.toml
  cp "$SRC/.jit/templates.toml" .jit/templates.toml

  MILESTONE_ID=$($JIT issue create "Demo release readiness" \
    --type milestone --label milestone:demo-release \
    --description $'Ship the demo release once its member epics land.\n\n## Success Criteria\n\n- [ ] [hard] REQ-01 Both member epics reach done.\n' \
    --json | jq -r '.short_id')
  EPIC1_ID=$($JIT issue create "Ingest pipeline" \
    --type epic --label epic:ingest --label milestone:demo-release \
    --description $'Build the ingest pipeline for the demo release.\n\n## Success Criteria\n\n- [ ] [hard] REQ-01 Records are ingested and persisted.\n' \
    --json | jq -r '.short_id')
  EPIC2_ID=$($JIT issue create "Query surface" \
    --type epic --label epic:query --label milestone:demo-release \
    --description $'Expose the query surface for the demo release.\n\n## Success Criteria\n\n- [ ] [hard] REQ-01 Queries return the persisted records.\n' \
    --json | jq -r '.short_id')
  # Containment: the milestone depends on its member epics (milestone -> epic),
  # the same edge direction this repo's own v1.0 milestone uses.
  $JIT dep add "$MILESTONE_ID" "$EPIC1_ID" "$EPIC2_ID" >/dev/null
  ;;

*)
  echo "Unknown scenario: $SCENARIO" >&2
  exit 1
  ;;
esac

git add -A && git commit -qm "chore: set up $SCENARIO test repo"

echo "Test repo ready at: $TARGET (scenario: $SCENARIO)"
if [ "$SCENARIO" = "routing" ]; then
  # Last line: the mode-1 prompt's {CONTAINER_ID} placeholder resolves to this.
  echo "MILESTONE_ID=$MILESTONE_ID"
fi
