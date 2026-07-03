#!/usr/bin/env bash
# Creates an isolated test repo for jit-project-lead SKELETON evals.
# Usage: ./setup-test-repo.sh <target-dir> <scenario>
# Scenarios: "two-tier" | "collapsed" | "fallback"
#
# These scenarios' observable behavior is pre-flight + tier derivation + the
# front-door stop-and-ask (a generic prompt has no explicit mode signal, so the
# front door asks which mode); it does not mutate .jit/ issue state. So each repo
# only needs a valid .jit/ (config + optional templates) and one git commit so
# `jit recover` (pre-flight step 2) succeeds. The canonical content-standards
# doc the skill reads is resolved from the jit repo through the ~/.claude/skills
# symlink, not from this test repo.
set -euo pipefail

JIT=/home/vkaskivuo/.cargo/bin/jit
SRC=/home/vkaskivuo/Projects/just-in-time      # source of the in-repo rulesets
TARGET="${1:?Usage: setup-test-repo.sh <target-dir> <scenario>}"
SCENARIO="${2:?Scenario required: two-tier | collapsed | fallback}"

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

*)
  echo "Unknown scenario: $SCENARIO" >&2
  exit 1
  ;;
esac

git add -A && git commit -qm "chore: set up $SCENARIO test repo"

echo "Test repo ready at: $TARGET (scenario: $SCENARIO)"
