# Task Classifier

Classify each issue into one of four categories to determine which agent prompt template to use for dispatch. Classification is based on signals from the issue's title, description, type label, and linked documents.

## Categories

### design
Architecture, API surface, system design, or interface definition work. The output is a design document or specification, not implementation code.

**Signals:**
- Title/description contains: "design", "architect", "API surface", "interface", "schema", "data model", "specification", "RFC", "proposal"
- Issue has `type:story` or a linked design doc that needs to be *created* (not just followed)
- Description asks for trade-off analysis or approach evaluation

**Agent prompt:** `references/architect-agent-prompt.md`

### research
Investigation, exploration, or spike work. The output is findings and recommendations, not implementation.

**Signals:**
- Title/description contains: "investigate", "spike", "explore", "research", "figure out", "evaluate", "benchmark", "compare", "assess", "unknown", "feasibility"
- The issue is about answering a question rather than producing a deliverable
- Description contains open-ended questions or "determine whether"

**Agent prompt:** `references/explorer-agent-prompt.md`

### documentation
Writing or updating documentation, content, or user-facing material. The output is text/docs, not code.

**Signals:**
- Title/description contains: "document", "docs", "README", "guide", "tutorial", "reference docs", "changelog", "release notes", "content"
- Issue has `type:task` with a documentation-focused description
- The deliverable is explicitly a document or content piece

**Agent prompt:** `references/doc-agent-prompt.md`

### implementation
The default category for leaf work items. Any issue that doesn't match the above patterns. The output is working implementation (code, configuration, assets, etc.) with passing tests/gates.

**Signals:**
- `type:task`, `type:bug`, or `type:enhancement` without design/research/doc signals
- Description focuses on building, fixing, or changing something concrete
- Has specific acceptance criteria about behavior or output

**Agent prompt:** Use jit-parallel's `agent-prompt-template.md` (at `.claude/skills/jit-parallel/references/agent-prompt-template.md`)

## Classification Priority

When an issue matches multiple categories, use this priority:
1. `research` — if the primary goal is to answer a question, classify as research even if it involves writing some code
2. `design` — if the primary output is a design/specification document
3. `documentation` — if the primary output is user-facing documentation
4. `implementation` — default fallback

## Example Classifications

| Title | Classification | Reasoning |
|---|---|---|
| "Investigate caching strategies for API responses" | research | "Investigate" + open-ended question |
| "Design the authentication flow" | design | "Design" + output is a specification |
| "Implement JWT token validation" | implementation | Concrete deliverable, no investigation |
| "Write API reference documentation" | documentation | "documentation" + output is docs |
| "Fix race condition in claim coordinator" | implementation | Bug fix, concrete deliverable |
| "Evaluate whether SQLite or JSON storage is better" | research | "Evaluate" + comparison question |
| "Define the event schema for v2" | design | "Define" + schema specification |
| "Add getting started guide to docs/" | documentation | "guide" + docs output |
