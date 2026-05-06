# Sub-Agent Prompt Template

Fill in the bracketed fields for each dispatched agent. Remove sections that don't apply.

---

You are [implementing / planning / reviewing] issue [SHORT-ID] in the JIT repository at /home/vkaskivuo/Projects/just-in-time.

## Issue

**Title:** [TITLE]
**ID:** [FULL-ID]

[FULL DESCRIPTION — paste verbatim from jit issue show]

## [For implementation tasks] What to do

1. [Derive concrete steps from the issue description and acceptance criteria.]
2. Write tests first (TDD). Run them to confirm they fail, then implement.
3. Ensure that the full test suite passes when done (per the project's testing conventions).
4. Check that the implementation is sufficient to meet all acceptance criteria and that it can pass all the quality gates defined in the issue.

## [For planning tasks] What to do

1. Explore the codebase to understand the current state — find relevant files, existing patterns, and constraints.
2. Write a concrete implementation plan to `dev/plans/[SHORT-ID]-[slug].md`. The plan must include:
   - Problem statement and goals
   - Ordered implementation steps
   - Files to create or modify (with rationale)
   - Key design decisions and trade-offs considered
   - TDD approach: concrete test names to write first
   - Acceptance criteria (refine or define if missing from the issue)
   - Any risks or unknowns that need resolution before implementation begins
3. Link the document to the issue using `jit doc add`:
   - id: "[SHORT-ID]"
   - path: "dev/plans/[SHORT-ID]-[slug].md"
   - doc_type: "implementation-plan"
   - label: "Implementation Plan"
4. Commit the plan file (and any updated `.jit/` files) so the document has a commit hash. Use a commit message that references the issue and the plan adhering to the project's commit message conventions.
5. Do NOT write implementation code. Return the plan path and a summary of key decisions.
6. The saved plan will be reviewed and fed into a subsequent implementation agent.

## [For review tasks] What to do

1. Locate the relevant code (search for key symbols from the issue description).
2. Verify each acceptance/success criterion is met.
3. Run testing, linting, and formatting checks per the project's coding conventions.
4. If complete: return just a confirmation that the issue is complete and ready to close. If incomplete: return a detailed description of what is missing or incorrect, referencing specific code locations and test results.

## Return

Return a summary of:
- Files modified or created
- Tests added (names)
- Confirmation of passing testing, linting, and formatting checks
- Any issues encountered
