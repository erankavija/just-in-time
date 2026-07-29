# Production Polish Design

## Overview

This document outlines quality-of-life improvements for v1.0: gate templates/presets and web UI state transition buttons. These features improve usability and reduce friction for both human users and AI agents.

## Motivation

Production systems benefit from:
- **Reduced boilerplate**: Common configurations should be one command
- **Best practices**: Templates encode proven patterns
- **Usability**: UI should support common actions directly

While not strictly required for functionality, these polish features significantly improve the user experience and reduce onboarding time.

## 1. Gate Templates/Presets

### Design Goals

Enable users and agents to quickly set up quality gates using battle-tested configurations for common languages and workflows, reducing setup time from minutes to seconds.

### Command Interface

```bash
# List available presets
jit gate preset list

# Show preset details
jit gate preset show rust-tdd

# Apply preset to issue
jit gate preset apply <issue-id> rust-tdd

# Create custom preset from existing gates
jit gate preset create my-workflow --from-issue <issue-id>

# Apply with customization
jit gate preset apply <issue-id> rust-tdd --timeout 120 --no-precheck
```

### Built-in Presets

#### Rust TDD Workflow

```json
{
  "name": "rust-tdd",
  "description": "Test-driven development workflow for Rust projects",
  "gates": [
    {
      "key": "tdd-reminder",
      "title": "Write tests first (TDD)",
      "stage": "precheck",
      "mode": "manual",
      "description": "Reminder to write failing tests before implementation"
    },
    {
      "key": "tests",
      "title": "All tests pass",
      "stage": "postcheck",
      "mode": "automated",
      "checker": {
        "type": "exec",
        "command": "cargo test",
        "timeout_seconds": 60
      }
    },
    {
      "key": "clippy",
      "title": "Clippy lints pass",
      "stage": "postcheck",
      "mode": "automated",
      "checker": {
        "type": "exec",
        "command": "cargo clippy -- -D warnings",
        "timeout_seconds": 30
      }
    },
    {
      "key": "fmt",
      "title": "Code formatted",
      "stage": "postcheck",
      "mode": "automated",
      "checker": {
        "type": "exec",
        "command": "cargo fmt -- --check",
        "timeout_seconds": 10
      }
    }
  ]
}
```

#### Python TDD Workflow

```json
{
  "name": "python-tdd",
  "description": "Test-driven development workflow for Python projects",
  "gates": [
    {
      "key": "tdd-reminder",
      "title": "Write tests first (TDD)",
      "stage": "precheck",
      "mode": "manual"
    },
    {
      "key": "pytest",
      "title": "All tests pass",
      "stage": "postcheck",
      "mode": "automated",
      "checker": {
        "type": "exec",
        "command": "pytest",
        "timeout_seconds": 60
      }
    },
    {
      "key": "black",
      "title": "Code formatted (Black)",
      "stage": "postcheck",
      "mode": "automated",
      "checker": {
        "type": "exec",
        "command": "black --check .",
        "timeout_seconds": 10
      }
    },
    {
      "key": "mypy",
      "title": "Type checking passes",
      "stage": "postcheck",
      "mode": "automated",
      "checker": {
        "type": "exec",
        "command": "mypy .",
        "timeout_seconds": 30
      }
    }
  ]
}
```

#### JavaScript/TypeScript TDD Workflow

```json
{
  "name": "js-tdd",
  "description": "Test-driven development workflow for JavaScript/TypeScript",
  "gates": [
    {
      "key": "tdd-reminder",
      "title": "Write tests first (TDD)",
      "stage": "precheck",
      "mode": "manual"
    },
    {
      "key": "jest",
      "title": "All tests pass",
      "stage": "postcheck",
      "mode": "automated",
      "checker": {
        "type": "exec",
        "command": "npm test",
        "timeout_seconds": 60
      }
    },
    {
      "key": "eslint",
      "title": "ESLint passes",
      "stage": "postcheck",
      "mode": "automated",
      "checker": {
        "type": "exec",
        "command": "npm run lint",
        "timeout_seconds": 30
      }
    },
    {
      "key": "prettier",
      "title": "Code formatted (Prettier)",
      "stage": "postcheck",
      "mode": "automated",
      "checker": {
        "type": "exec",
        "command": "npm run format:check",
        "timeout_seconds": 10
      }
    }
  ]
}
```

#### Security Audit Workflow

```json
{
  "name": "security-audit",
  "description": "Security review workflow",
  "gates": [
    {
      "key": "security-review",
      "title": "Security review completed",
      "stage": "precheck",
      "mode": "manual",
      "description": "Review code for security vulnerabilities: injection, auth, crypto, secrets"
    },
    {
      "key": "secret-detection",
      "title": "No secrets in code",
      "stage": "postcheck",
      "mode": "automated",
      "checker": {
        "type": "exec",
        "command": "gitleaks detect --no-git",
        "timeout_seconds": 20
      }
    },
    {
      "key": "dependency-audit",
      "title": "Dependency vulnerabilities checked",
      "stage": "postcheck",
      "mode": "automated",
      "checker": {
        "type": "exec",
        "command": "cargo audit",
        "timeout_seconds": 30
      }
    }
  ]
}
```

#### Minimal Workflow

```json
{
  "name": "minimal",
  "description": "Minimal workflow with just code review",
  "gates": [
    {
      "key": "code-review",
      "title": "Code review completed",
      "stage": "postcheck",
      "mode": "manual"
    }
  ]
}
```

### Preset Storage

Presets stored in two locations:
1. **Built-in**: Bundled with binary (embedded in Rust code)
2. **Custom**: User-defined in `.jit/config/gate-presets/`

Custom presets override built-in with same name.

### Customization Options

When applying preset, support parameter overrides:
- `--timeout <seconds>`: Override checker timeouts
- `--no-precheck`: Skip precheck gates
- `--no-postcheck`: Skip postcheck gates
- `--only <gate-key>`: Apply only specific gates from preset
- `--except <gate-key>`: Apply all except specific gates

### Implementation Approach

1. **Preset Schema**: Define JSON schema for presets
2. **Embedded Presets**: Include built-in presets in binary (Rust `include_str!`)
3. **Preset Manager**: Load from both built-in and custom locations
4. **CLI Commands**: `list`, `show`, `apply`, `create`
5. **Validation**: Validate presets on load
6. **Documentation**: Auto-generate preset reference from schemas

### Testing Strategy

- Unit tests: Preset parsing and validation
- Integration tests: Apply preset to issue, verify gates created
- CLI tests: All preset commands
- Documentation tests: Examples in docs work correctly

## 2. Web UI State Transition Buttons

### Design Goals

Enable users to transition issue states directly from the web UI, improving usability and reducing context switching to CLI.

### Current State

Web UI is currently read-only:
- View issues and dependencies
- View documents and search results
- View strategic/tactical hierarchies
- Filter by labels

Missing: Ability to modify issue state

### Target Functionality

Add interactive controls for common operations:

#### Issue Detail View

**State Transition Buttons**:
- "Start Work" (ready → in-progress): Atomic claim + state change
- "Submit for Review" (in-progress → gated): Trigger postchecks
- "Mark Done" (gated → done): Only if all gates pass
- "Block Issue" (any → blocked): Prompt for reason
- "Unblock Issue" (blocked → previous state)
- "Return to Backlog" (ready/in-progress → backlog)

**Assignment Controls**:
- "Claim Issue" (if unassigned)
- "Unclaim Issue" (if self-assigned)
- "Reassign" (if assigned to others, admin only)

**Gate Controls**:
- "Run Precheck" button (triggers gate preview)
- "Run Postcheck" button (triggers full gate check)
- "View Gate Results" (link to latest run details)

#### Bulk Operations (List View)

**Multi-select Actions**:
- Select multiple issues (checkboxes)
- Bulk state change
- Bulk label add/remove
- Bulk assign

### UI Design

#### State Transition Flow

```
┌─────────────────────────────────────┐
│ Issue: Implement Feature X          │
│ State: ready                        │
│ Priority: high                      │
│                                     │
│ ┌──────────────┐  ┌──────────────┐ │
│ │ Start Work   │  │ Move to      │ │
│ └──────────────┘  │ Backlog      │ │
│                   └──────────────┘ │
└─────────────────────────────────────┘
```

On click:
1. Show confirmation dialog (for destructive actions)
2. Send API request to backend
3. Update UI optimistically (with rollback on error)
4. Show toast notification (success/error)

#### Gate Status Display

```
┌─────────────────────────────────────┐
│ Quality Gates                       │
│                                     │
│ ✓ tests          passed   2m ago   │
│ ✓ clippy         passed   2m ago   │
│ ✗ code-review    pending  -        │
│                                     │
│ ┌──────────────────┐               │
│ │ Run All Checks   │               │
│ └──────────────────┘               │
└─────────────────────────────────────┘
```

### Backend API

Add REST endpoints to web server:

```
POST /api/issues/:id/transition
  Body: { "to_state": "in_progress", "assignee": "user:alice" }
  Response: { "issue": {...}, "success": true }

POST /api/issues/:id/claim
  Body: { "assignee": "user:alice" }
  Response: { "issue": {...}, "success": true }

POST /api/issues/:id/gates/run
  Body: { "stage": "precheck" | "postcheck" | "all" }
  Response: { "results": [...], "success": true }

POST /api/issues/bulk
  Body: { "issue_ids": [...], "operation": "update", "changes": {...} }
  Response: { "modified": 5, "failed": 0, "errors": [] }
```

### Implementation Approach

#### Frontend (TypeScript/React)

1. **State Management**: Add mutation functions (React Query or similar)
2. **Button Components**: Reusable state transition buttons
3. **Confirmation Dialogs**: Modal dialogs for destructive actions
4. **Optimistic Updates**: Update UI immediately, rollback on error
5. **Error Handling**: Toast notifications for errors
6. **Loading States**: Disable buttons during API calls

#### Backend (Rust)

1. **API Endpoints**: Add handlers for state transitions
2. **Validation**: Verify transitions are valid (same as CLI)
3. **Authentication**: Basic auth or token-based (future: proper RBAC)
4. **Error Responses**: Structured JSON errors
5. **CORS**: Enable for local development

### Security Considerations

**Short-term** (v1.0):
- No authentication (trust local network)
- Document security implications
- Recommend firewall rules

**Future** (v1.1+):
- Add authentication (JWT tokens)
- Role-based access control (RBAC)
- Audit log for UI actions

### Testing Strategy

- Unit tests: Button components, state management
- Integration tests: API endpoints
- E2E tests: Full workflow (Playwright or Cypress)
- Accessibility tests: Keyboard navigation, screen readers

### Documentation

- User guide: Web UI operations walkthrough
- Security guide: Deployment recommendations
- Developer guide: Adding new operations

## Implementation Plan

### Gate Templates/Presets

**Day 1-2**: Preset schema, built-in presets, preset manager
**Day 3**: CLI commands (list, show, apply, create)
**Day 4**: Testing and documentation

**Total**: 3-4 days

### Web UI State Transitions

**Day 1-2**: Backend API endpoints, validation
**Day 3-4**: Frontend components, state management
**Day 5-6**: Integration, testing, polish
**Day 7**: Documentation

**Total**: 6-7 days

**Combined effort**: 9-11 days for both features
