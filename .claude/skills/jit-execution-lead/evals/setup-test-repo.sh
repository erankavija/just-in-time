#!/usr/bin/env bash
# Creates an isolated test repo with JIT initialized for eval purposes.
# Usage: ./setup-test-repo.sh <target-dir> <scenario>
# Scenarios: "sw-with-children", "sw-needs-breakdown", "content-project"
set -euo pipefail

JIT=/home/vkaskivuo/.cargo/bin/jit
TARGET="${1:?Usage: setup-test-repo.sh <target-dir> <scenario>}"
SCENARIO="${2:?Scenario required: sw-with-children | sw-needs-breakdown | content-project}"

rm -rf "$TARGET"
mkdir -p "$TARGET"
cd "$TARGET"

git init -b main
git config user.name "Test User"
git config user.email "test@example.com"

# Initialize JIT
$JIT init

case "$SCENARIO" in

sw-with-children)
  # A small software project with conventions, an epic, and pre-created children.
  cat > CLAUDE.md << 'CLAUDEEOF'
# Test Project

A small utility library.

## Conventions
- Use Python 3.12+
- Follow PEP 8 style
- All public functions must have docstrings
- Tests use pytest, placed in tests/ directory
- Type hints required on all function signatures
CLAUDEEOF

  mkdir -p src tests
  cat > src/__init__.py << 'EOF'
"""Test project utility library."""
EOF
  cat > tests/__init__.py << 'EOF'
EOF

  git add -A && git commit -m "Initial commit"

  # Define quality gates in registry
  $JIT gate define tests --title "Tests" --description "All pytest tests pass" --mode auto --checker-command "cd '$TARGET' && python -m pytest tests/ -q"
  $JIT gate define code-review --title "Code Review" --description "Code review by lead" --mode manual

  # Create epic
  EPIC_ID=$($JIT issue create \
    --title "String Utilities Epic" \
    --description "$(cat <<'DESC'
Implement a set of string utility functions for the library.

## Success Criteria
- [ ] A `slugify` function that converts strings to URL-safe slugs
- [ ] A `truncate` function that shortens strings with ellipsis
- [ ] All functions have docstrings and type hints
- [ ] All functions have pytest tests with edge cases
DESC
)" \
    --label "type:epic" \
    --priority normal \
    --json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")

  # Create child tasks
  TASK1_ID=$($JIT issue create \
    --title "Implement slugify function" \
    --description "$(cat <<'DESC'
Create a `slugify(text: str) -> str` function in `src/strings.py`.

## Success Criteria
- [ ] Converts spaces to hyphens
- [ ] Removes non-alphanumeric characters (except hyphens)
- [ ] Converts to lowercase
- [ ] Handles empty strings gracefully
- [ ] Has pytest tests in tests/test_strings.py
DESC
)" \
    --label "type:task" --label "epic:string-utils" \
    --priority normal \
    --gate tests --gate code-review \
    --json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")

  TASK2_ID=$($JIT issue create \
    --title "Implement truncate function" \
    --description "$(cat <<'DESC'
Create a `truncate(text: str, max_length: int, suffix: str = '...') -> str` function in `src/strings.py`.

## Success Criteria
- [ ] Returns original string if shorter than max_length
- [ ] Truncates and appends suffix if longer
- [ ] Handles edge cases: empty string, max_length < len(suffix)
- [ ] Has pytest tests in tests/test_strings.py
DESC
)" \
    --label "type:task" --label "epic:string-utils" \
    --priority normal \
    --gate tests --gate code-review \
    --json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")

  # Wire dependencies: task2 depends on task1 (shared file), epic depends on both
  $JIT dep add "$TASK2_ID" "$TASK1_ID"
  $JIT dep add "$EPIC_ID" "$TASK1_ID" "$TASK2_ID"

  # Add epic label
  $JIT issue update "$EPIC_ID" --label "epic:string-utils"

  git add .jit && git commit -m "chore: set up epic with children"

  echo "EPIC_ID=$EPIC_ID"
  echo "TASK1_ID=$TASK1_ID"
  echo "TASK2_ID=$TASK2_ID"
  ;;

sw-needs-breakdown)
  # A software project with an epic that has a design doc but no children.
  cat > CLAUDE.md << 'CLAUDEEOF'
# Calculator Service

A simple REST calculator API.

## Conventions
- Python 3.12+ with FastAPI
- Tests use pytest
- All endpoints must have OpenAPI docstrings
- Error responses use standard HTTP codes with JSON body
CLAUDEEOF

  mkdir -p src tests docs
  git add -A && git commit -m "Initial commit"

  $JIT gate define tests --title "Tests" --description "All pytest tests pass" --mode auto --checker-command "python -m pytest tests/ -q"
  $JIT gate define code-review --title "Code Review" --description "Code review" --mode manual

  EPIC_ID=$($JIT issue create \
    --title "Basic Arithmetic Endpoints" \
    --description "$(cat <<'DESC'
Implement the four basic arithmetic operations as REST endpoints.

## Success Criteria
- [ ] POST /add endpoint that accepts two numbers and returns their sum
- [ ] POST /subtract endpoint
- [ ] POST /multiply endpoint
- [ ] POST /divide endpoint with proper division-by-zero handling
- [ ] All endpoints have OpenAPI documentation
- [ ] All endpoints have pytest tests including edge cases
DESC
)" \
    --label "type:epic" \
    --priority high \
    --gate tests --gate code-review \
    --json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")

  $JIT issue update "$EPIC_ID" --label "epic:basic-arithmetic"

  # Create a design doc
  mkdir -p dev/active
  SHORT_ID=$(echo "$EPIC_ID" | cut -c1-8)
  cat > "dev/active/${SHORT_ID}-basic-arithmetic.md" << 'DOCEOF'
# Basic Arithmetic Endpoints

**Issue:** see epic
**Type:** epic

## Problem Statement
The calculator service needs four basic arithmetic REST endpoints.

## Design
Each endpoint accepts a JSON body `{"a": number, "b": number}` and returns `{"result": number}`.

Endpoints:
- POST /add → a + b
- POST /subtract → a - b
- POST /multiply → a * b
- POST /divide → a / b (returns 400 if b == 0)

All endpoints share a common request/response model defined in `src/models.py`.

## Implementation Steps
1. Create request/response models in src/models.py
2. Implement each endpoint in src/routes.py
3. Add error handling for division by zero
4. Write tests for each endpoint
5. Add OpenAPI docstrings

## Testing Approach
Use pytest with FastAPI TestClient. Test happy paths and edge cases (large numbers, zero, negative numbers, division by zero).
DOCEOF

  $JIT doc add "$EPIC_ID" "dev/active/${SHORT_ID}-basic-arithmetic.md" --doc-type design --label "Design Document"

  git add -A && git commit -m "chore: set up epic with design doc"

  echo "EPIC_ID=$EPIC_ID"
  ;;

content-project)
  # A non-software project: creating a documentation/content set.
  cat > CLAUDE.md << 'CLAUDEEOF'
# Product Documentation Project

Documentation for a fictional product called "Nexus".

## Conventions
- Use Markdown for all content
- Follow Diataxis structure: tutorials, how-to guides, reference, explanation
- Tone: professional but approachable, second person ("you")
- Maximum heading depth: H3
- All docs must have a "Prerequisites" section if applicable
- File naming: lowercase-with-hyphens.md
CLAUDEEOF

  mkdir -p docs/tutorials docs/how-to docs/reference docs/explanation
  cat > docs/index.md << 'EOF'
# Nexus Documentation

Welcome to the Nexus documentation.
EOF

  git add -A && git commit -m "Initial commit"

  $JIT gate define content-review --title "Content Review" --description "Content review by lead" --mode manual

  EPIC_ID=$($JIT issue create \
    --title "Getting Started Documentation Set" \
    --description "$(cat <<'DESC'
Create the initial getting started documentation for Nexus.

## Success Criteria
- [ ] A tutorial: "Your First Nexus Project" in docs/tutorials/
- [ ] A how-to guide: "How to Configure Nexus" in docs/how-to/
- [ ] A reference page: "Configuration Options" in docs/reference/
- [ ] All docs follow the Diataxis structure and project conventions
- [ ] docs/index.md is updated with links to new content
DESC
)" \
    --label "type:epic" \
    --priority high \
    --gate content-review \
    --json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")

  $JIT issue update "$EPIC_ID" --label "epic:getting-started"

  git add .jit && git commit -m "chore: set up documentation epic"

  echo "EPIC_ID=$EPIC_ID"
  ;;

*)
  echo "Unknown scenario: $SCENARIO"
  exit 1
  ;;
esac

echo "Test repo ready at: $TARGET"
