#!/usr/bin/env bash
# Creates an isolated test repo with JIT + the plan-before-fan-out bracket wired up,
# for evaluating the jit-planning-lead skill.
# Usage: ./setup-test-repo.sh <target-dir> <scenario>
# Scenarios: "research-and-plan", "plan-from-existing", "plan-from-import"
#
# Each planning entry path gets its own seed state:
#   research-and-plan   — no container exists; only a vague idea (supplied by the prompt).
#   plan-from-existing  — an epic container with [hard] REQ-NN criteria already exists.
#   plan-from-import    — an external planning doc exists in the repo; no container yet.
#
# The script reproduces the *default ruleset* the jit-planning-lead skill documents by
# copying the live config from a source JIT repo ($JIT_SRC): the `plan` template
# (.jit/templates.toml), the coverage rule (.jit/rules.toml), the type hierarchy and
# namespaces (.jit/config.toml), the rule schemas (.jit/schemas/), and the portable
# coverage-preview checker (scripts/coverage-preview.sh). It then defines the four
# bracket gates the `plan` template references. The two AI-review gates (plan-review,
# breakdown-review) are defined `manual` so an eval runner can attest them via
# `jit gate evaluate --by ...` without the production reviewer; coverage-preview and
# repo-validate are real `auto` gates (jit validate), so coverage keeps its teeth.
set -euo pipefail

JIT="${JIT:-/home/vkaskivuo/.cargo/bin/jit}"
JIT_SRC="${JIT_SRC:-/home/vkaskivuo/Projects/just-in-time}"
TARGET="${1:?Usage: setup-test-repo.sh <target-dir> <scenario>}"
SCENARIO="${2:?Scenario required: research-and-plan | plan-from-existing | plan-from-import}"

rm -rf "$TARGET"
mkdir -p "$TARGET"
cd "$TARGET"

git init -b main -q
git config user.name "Test User"
git config user.email "test@example.com"

# Initialize JIT, then overlay the default ruleset from the source repo so
# `jit apply plan` behaves exactly as in production.
$JIT init >/dev/null

cp "$JIT_SRC/.jit/templates.toml" .jit/templates.toml
cp "$JIT_SRC/.jit/rules.toml"     .jit/rules.toml
cp "$JIT_SRC/.jit/config.toml"    .jit/config.toml
# Dogfood projections target files absent from the portable eval repository.
sed -i '/^\[projection\./,$d' .jit/config.toml
mkdir -p .jit/schemas
cp -r "$JIT_SRC"/.jit/schemas/. .jit/schemas/ 2>/dev/null || true
mkdir -p scripts
cp "$JIT_SRC/scripts/coverage-preview.sh" scripts/coverage-preview.sh
chmod +x scripts/coverage-preview.sh

# Project conventions shared by all scenarios: a small Python utility library.
cat > AGENTS.md << 'AGENTEOF'
# Test Utility Library

A small Python utility library.

## Conventions
- Python 3.12+
- Follow PEP 8 style; type hints required on all public function signatures
- All public functions must have docstrings
- Tests use pytest, placed in tests/ directory
- Public API lives under src/
AGENTEOF
mkdir -p src tests
printf '"""Test utility library."""\n' > src/__init__.py
: > tests/__init__.py

# --- Bracket gates referenced by the `plan` template ---------------------------
# repo-validate: whole-repo validation (real, portable).
$JIT gate define repo-validate --title "Repo Validate" \
  --description "Whole-repository validation must pass (jit validate, no issue id)" \
  --mode auto --checker-command "jit validate" >/dev/null
# plan-review / breakdown-review: production uses a codex-based AI reviewer; defined
# manual here so the eval runner attests them after its own adversarial review pass.
$JIT gate define plan-review --title "Plan Review" \
  --description "Adversarial plan review before fan-out (attested by the planning lead)" \
  --mode manual >/dev/null
$JIT gate define breakdown-review --title "Breakdown Review" \
  --description "Adversarial breakdown review (attested by the planning lead)" \
  --mode manual >/dev/null
# coverage-preview: real scoped-coverage checker (jit validate --scope <C>).
$JIT gate define coverage-preview --title "Coverage Preview" \
  --description "Scoped coverage validation: every [hard] REQ credited by a satisfies: label" \
  --mode auto --checker-command "./scripts/coverage-preview.sh" >/dev/null

# --- Impl-tier gates the breakdown attaches to leaf work ------------------------
# Never driven during planning (execution does that); present so jit-breakdown can
# attach a standard gate tier to the impl children it creates.
$JIT gate define tests --title "Tests" \
  --description "All pytest tests pass" \
  --mode auto --checker-command "cd '$TARGET' && python -m pytest tests/ -q" >/dev/null
$JIT gate define code-review --title "Code Review" \
  --description "Code review by lead" --mode manual >/dev/null

git add -A && git commit -qm "chore: init test repo with plan bracket wired"

case "$SCENARIO" in

research-and-plan)
  # No container exists. The vague idea is supplied by the eval prompt; the skill's
  # research-and-plan path must refine it into a container before planning behind it.
  echo "SCENARIO=research-and-plan"
  echo "NOTE=no container issue exists; the idea is carried in the eval prompt"
  ;;

plan-from-existing)
  # An epic container with success criteria already exists; the skill seeds a plan
  # from it (plan-from-existing path). No bracket, no children yet.
  EPIC_ID=$($JIT issue create \
    --title "Retry Utility" \
    --description "$(cat <<'DESC'
Provide a reusable retry helper for flaky operations in the library.

## Success Criteria
- [hard] REQ-01: a `retry` decorator re-invokes the wrapped callable on failure
- [hard] REQ-02: configurable maximum attempts and per-attempt delay
- [hard] REQ-03: exponential backoff with an optional cap on the delay
- [hard] REQ-04: only caller-specified exception types are retried; others propagate
DESC
)" \
    --label "type:epic" \
    --priority normal \
    --json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")

  $JIT issue update "$EPIC_ID" --label "epic:retry-utility" >/dev/null
  git add -A && git commit -qm "chore: seed epic container with success criteria"
  echo "SCENARIO=plan-from-existing"
  echo "EPIC_ID=$EPIC_ID"
  ;;

plan-from-import)
  # An external planning document exists in the repo but is not yet a jit container.
  # The skill's plan-from-import path reconciles it into a container, then plans behind it.
  mkdir -p notes
  cat > notes/ttl-cache-design.md << 'DOCEOF'
# TTL Cache — Design Note (external)

We keep re-computing expensive lookups. This note sketches a small in-memory
time-to-live cache for the utility library. It has not been entered into jit yet.

## What we want
- A `TTLCache` object that stores key/value pairs, each with an expiry.
- Reading an expired entry must behave as a miss (the stale value is not returned).
- A configurable maximum size; when full, the least-recently-used entry is evicted.
- A `get_or_compute(key, fn)` convenience that returns the cached value or computes,
  stores, and returns it on a miss.
- Basic statistics: number of hits and misses, readable by the caller.

## Notes / open questions
- Expiry granularity is seconds; a per-entry TTL overrides a default TTL.
- Thread-safety is out of scope for the first version (single-threaded callers only).
DOCEOF

  git add -A && git commit -qm "chore: add external TTL cache design note"
  echo "SCENARIO=plan-from-import"
  echo "IMPORT_DOC=notes/ttl-cache-design.md"
  ;;

*)
  echo "Unknown scenario: $SCENARIO" >&2
  exit 1
  ;;
esac

echo "Test repo ready at: $TARGET"
