---
name: jit-manage
description: >
  JIT project management workflow orchestrator covering the full issue lifecycle:
  surveying available work, claiming and planning issues, creating new work items
  with DAG wiring, reviewing project status, and completing work with gate
  enforcement. Use when asked to "manage issues", "start a work session",
  "what should I work on", "create an issue/epic/story/task", "show project status",
  "complete this issue", or any general JIT project management question.
compatibility: Requires JIT CLI on PATH or JIT MCP tools available. Works in any repository with a .jit/ directory.
---

# JIT Project Management

Orchestrate the full JIT issue lifecycle: survey work, claim issues, plan,
implement, pass gates, and complete — with proper DAG wiring throughout.

## Invariants

These rules apply across **all** workflows. Never violate them.

1. **DAG is the foundation.** Every issue must participate in the dependency
   graph. A chain of dependencies defines work order. Leaf tasks depend on
   nothing; parents depend on their children. No orphaned issues.

2. **Success criteria are mandatory.** Every issue description must include
   an explicit `## Success Criteria` section with verifiable items. All
   criteria must be confirmed complete before closing the issue. No
   unjustified scope drift — work only on what the criteria specify. If scope
   must change, update the criteria first and record the justification.

3. **Labels aid grouping; DAG defines membership.** Use labels like
   `epic:auth` for filtering, but containment is expressed via dependency
   edges (parent depends on children).

4. **Claims before work.** An issue must be claimed before any work begins.

5. **Gates before completion.** All quality gates must pass before an issue
   can transition to `done`.

6. **Close before moving on.** Work on the next issue cannot begin until the
   current issue is closed (done or rejected).

7. **Separate chore commits.** JIT state changes (`.jit/`) are committed
   separately from code changes, batched per workflow step. See
   [references/state-commit-patterns.md](references/state-commit-patterns.md).

8. **Design docs in `dev/active/`.** Plans and design documents are saved to
   `dev/active/` and linked to the corresponding issue via `jit doc add`.

9. **Markdown everywhere.** Issue descriptions and plan documents use
   markdown format.

---

## Step 0: Pre-flight

Runs before every workflow. Do not skip.

1. Verify `.jit/` exists in the repo root. If not, suggest `jit init` or
   the `jit-migrate` skill and stop.

2. Run `jit recover` to clean stale locks from crashed processes.

3. Read `.jit/config.toml` to extract:
   - Type hierarchy (`[type_hierarchy].types`) — **never hardcode type names**
   - Label associations (`[type_hierarchy.label_associations]`)
   - Available label namespaces

4. Run `jit gate list --json` to learn what gates are configured.

5. Route to the appropriate workflow:

   | User intent | Workflow |
   |-------------|----------|
   | "what's available" / "start session" / "what should I work on" | **A** |
   | "work on X" / "claim" / "implement" | **B** |
   | "create issue" / "add task" / "new epic/story" | **C** |
   | "status" / "dashboard" / "what's blocked" / "progress" | **D** |
   | "finish" / "complete" / "close" / "pass gates" | **E** |

---

## Workflow A: Start a Work Session

Survey the project and select work.

1. Gather data in parallel:
   - `jit status --json` — overall counts
   - `jit query strategic --json` — epics and milestones
   - `jit query available --json` — ready, unassigned issues
   - `jit query blocked --json` — stuck issues with reasons

2. For each strategic issue, check progress:
   - `jit graph deps <id> --depth 0 --json` — count done vs total children

3. For each available issue, check for design docs:
   - `jit doc list <id> --json`

4. Present a formatted dashboard:
   ```
   PROJECT STATUS
   ==============
   Open: N | Ready: N | In Progress: N | Done: N | Blocked: N

   STRATEGIC ISSUES
   ----------------
   [type] Title (M/N children done, P%)

   AVAILABLE WORK (by priority)
   ----------------------------
   [priority] short-id  Title  (labels)  -- gates: X, Y  -- design doc: yes/no

   BLOCKED ISSUES
   --------------
   short-id  Title  -- blocked by: <reasons>
   ```

5. Rank available issues by priority. Show the top 5.

6. Ask the user which issue to work on:
   - Pick an issue -> transition to **Workflow B**
   - Create new work -> transition to **Workflow C**
   - Multiple independent issues available -> suggest the `jit-parallel` skill

---

## Workflow B: Work on an Issue

Full lifecycle from claim through completion.

### B1. Claim

1. `jit issue show <id> --json` — fetch full issue details.

2. Check state:
   - `ready` — proceed to claim
   - `backlog` — show blockers via `jit graph deps <id>`, explain what
     must complete first, and stop
   - `in_progress` — show current assignee; ask if user wants to release
     and reclaim

3. Claim the issue:
   ```bash
   jit issue claim <id> agent:claude
   ```
   Use the user-specified assignee if provided instead of `agent:claude`.

4. Commit JIT state:
   ```bash
   git add .jit && git commit -m "chore: claim issue <short-id> (<title>)"
   ```

### B2. Understand

1. Display the issue's full details: title, description, labels, gates
   required, and linked documents.

2. **Verify success criteria exist.** Check that the description contains a
   `## Success Criteria` section (or equivalent: "Acceptance Criteria",
   "Definition of Done"). If missing:
   - Warn: "This issue has no success criteria. Work cannot begin without
     them."
   - Draft criteria based on the title and description.
   - Ask the user to confirm or edit.
   - Update the issue: `jit issue update <id> --description "<updated>"`.
   - Commit JIT state.

3. If design docs are linked, read them via `jit doc show <id> <path>`.

4. If no design doc exists and the issue is non-trivial (story or above in
   the type hierarchy, or description mentions "design" / "plan" /
   "investigate"), offer to create one — proceed to B3.

### B3. Plan (optional)

1. Create a design document using the template from
   [references/design-doc-template.md](references/design-doc-template.md).
   Include the issue's success criteria in the document.

2. Save to `dev/active/<short-id>-<slug>.md` (derive slug from title:
   lowercase, hyphens, max 30 chars).

3. Commit the plan file:
   ```bash
   git add dev/active/<short-id>-<slug>.md
   git commit -m "docs: add plan for <short-id>"
   ```

4. Link to the issue:
   ```bash
   jit doc add <id> dev/active/<short-id>-<slug>.md \
     --doc-type design --label "Design Document"
   ```

5. Commit JIT state:
   ```bash
   git add .jit && git commit -m "chore: link plan to issue <short-id>"
   ```

6. Present the plan for user review before proceeding.

### B4. Pre-implementation checklist

Before implementation begins, verify and present all items:

- [ ] Issue is claimed and in `in_progress` state
- [ ] Success criteria are explicit, verifiable, and understood
- [ ] Design doc has been read (or created) and approach is understood
- [ ] All dependencies are `done` (re-check with `jit graph deps <id>`)
- [ ] Required gates are known — list each with type (auto/manual)
- [ ] Project coding conventions from CLAUDE.md are understood

Then **defer to the project's own conventions** for actual implementation.
The skill does not prescribe implementation methodology — that is
project-specific.

**Scope guard:** During implementation, work only on what the success
criteria specify. If you discover additional work is needed:
1. Stop implementation.
2. Propose updated success criteria with justification.
3. Get user approval before continuing.
4. Update the issue description with the revised criteria and commit.

### B5. Complete

Delegates to **Workflow E**.

---

## Workflow C: Create New Work

Create issues with proper DAG wiring, labels, and success criteria.

### C1. Determine scope

1. Read the type hierarchy to know valid types and their levels.
2. Route:
   - Single leaf issue (task/bug) -> **C2**
   - Higher-level issue (epic/story) -> **C3**
   - Batch from plan document -> **C4**

### C2. Create a single issue

1. Gather from the user:
   - **Title** — concise, action-oriented
   - **Description** — follow `.claude/skills/jit-manage/references/content-standards.md`:
     standalone markdown document, `## Success Criteria` with verifiable items,
     Mermaid for diagrams, LaTeX for math
   - **Type** — from configured hierarchy
   - **Priority** — critical / high / normal / low

2. Determine parent membership:
   - Search existing parents: `jit query all --label "type:epic" --json`
     (or the appropriate parent type from the hierarchy)
   - Ask the user which parent this belongs under
   - Derive membership label from `[type_hierarchy.label_associations]`
     (e.g., `epic:user-auth`)

3. Determine gates: inspect sibling issues for consistent gate assignment,
   or ask the user.

4. Create the issue:
   ```bash
   jit issue create \
     --title "<title>" \
     --description "<description with success criteria>" \
     --label "type:<type>" \
     --label "<membership-label>" \
     --priority <priority> \
     --gate <gate1> --gate <gate2>
   ```

5. Wire into the DAG:
   - Sequencing deps (if any): `jit dep add <new-id> <blocker-ids...>`
   - Containment edge: `jit dep add <parent-id> <new-id>`

6. Validate: the issue must have at least one dependency edge. If orphaned,
   warn and ask the user to wire it or confirm it as a root.

7. Commit JIT state:
   ```bash
   git add .jit && git commit -m "chore: create issue <short-id> (<title>)"
   ```

8. Run `jit validate` to confirm repository integrity.

### C3. Create a higher-level issue with children

1. Create the parent issue (same as C2 but with the higher-level type).

2. If the user wants to break it down into children immediately, suggest the
   `jit-breakdown` skill.

3. If children are provided inline:
   - Create each child issue with the membership label and type
   - Wire containment: `jit dep add <parent-id> <child-id>` for each child
   - Wire sequencing between siblings where order matters

4. Single batch commit for all JIT state:
   ```bash
   git add .jit && git commit -m "chore: create <N> issues for <parent-title>"
   ```

### C4. Batch creation from a plan document

1. Dispatch a `general-purpose` sub-agent using the prompt template at
   [references/issue-extraction-prompt.md](references/issue-extraction-prompt.md).

   Fill in the template fields:

   | Field | Value |
   |---|---|
   | `[PLAN_DOCUMENT_PATH]` | Absolute path to the plan document |
   | `[TYPE_HIERARCHY_TABLE]` | From config.toml |
   | `[EXISTING_EPICS_LIST]` | Current epics/stories for membership wiring |
   | `[EXISTING_LABELS_LIST]` | Known label namespaces |

2. Parse the returned JSON. Present the plan for review:
   ```
   Batch creation plan  (N issues, M dependency edges)

     [type] Title (ref: A, priority: high)   (no deps)
     [type] Title (ref: B, priority: normal) <- depends on A
     ...

   Create these N issues with M dependency edges? [yes / edit / abort]
   ```

3. On approval, execute in topological order:
   - Create all issues (deps-first)
   - Wire sequencing edges between peers
   - Wire containment edges to parents

4. Commit JIT state:
   ```bash
   git add .jit && git commit -m "chore: create <N> issues from plan"
   ```

5. Run `jit validate`.

---

## Workflow D: Review Project Status

Dashboard with actionable insights.

1. Gather data:
   - `jit status --json` — counts by state
   - `jit query strategic --json` — epics/milestones
   - `jit query blocked --json` — blocking analysis
   - `jit query all --state in_progress --json` — active work

2. For each strategic issue, compute progress:
   - `jit graph deps <id> --depth 0 --json` — count done vs total children

3. Present the dashboard (same format as Workflow A step 4).

4. Offer follow-up actions:
   - Pick up available work -> **Workflow A**
   - Unblock something (show specific blockers)
   - Create new work -> **Workflow C**

---

## Workflow E: Complete Work

Gate enforcement, success criteria verification, state transition, commit.

1. Identify the issue to complete (from context or ask the user).

2. Verify preconditions:
   - State is `in_progress` (or `gated` from a prior attempt)
   - Assigned to the current agent

3. **Verify success criteria are met.** Read the issue description's
   `## Success Criteria` section. For each criterion:
   - Determine if it can be verified automatically (test output, file
     existence, etc.) or requires manual confirmation.
   - Present each criterion with its status:
     ```
     SUCCESS CRITERIA
     ----------------
     [x] Criterion 1 — verified: <evidence>
     [x] Criterion 2 — verified: <evidence>
     [ ] Criterion 3 — UNMET: <what's missing>
     ```
   - If any criterion is unmet, **do not proceed**. Either:
     - Fix the gap and re-verify, or
     - If the criterion is no longer relevant, propose removing it with
       justification and get user approval before updating.

4. Run automated gates:
   ```bash
   jit gate check-all <id> --json
   ```

5. Handle gate results:
   - All pass -> proceed to step 7
   - Auto gate failure -> show output, offer to fix and retry
   - Manual gate pending -> prompt user, then:
     ```bash
     jit gate pass <id> <gate-key>
     ```

6. If any gate still fails, loop back to step 4 after fixing.

7. Transition to done:
   ```bash
   jit issue update <id> --state done
   ```
   If exit code 4 (gated — pending gates), show which gates remain and
   loop to step 4.

8. Commit JIT state:
   ```bash
   git add .jit && git commit -m "chore: complete issue <short-id> (<title>)"
   ```

9. Check cascade — see if completing this issue unblocked dependents:
   ```bash
   jit graph downstream <id> --json
   ```
   Report any issues that transitioned to `ready`.

10. Run `jit validate` to confirm repository integrity.
