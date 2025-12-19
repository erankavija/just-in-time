# AI Agent Validation Design

## Overview

This document outlines the comprehensive validation strategy for ensuring the just-in-time issue tracker works effectively with AI agents (particularly LLM-based agents like Claude and GitHub Copilot). This validation is critical for v1.0 as the system is explicitly designed for programmatic agent orchestration.

## Motivation

While the system has been developed with agent-first principles (JSON output, atomic operations, clear state machines), it has not been systematically validated with real AI agents in realistic workflows. Production readiness requires empirical evidence that:

1. AI agents can understand and use the gate system effectively
2. MCP server tools provide the right abstractions
3. Error messages are actionable for agents
4. Workflows are efficient and don't require excessive token usage
5. Documentation is sufficient for agent onboarding

## Validation Dimensions

### 1. Gate System Usability

**Objective**: Verify AI agents can define, manage, and respond to quality gates.

**Test Scenarios**:

- **Gate Definition**: Can agent create appropriate gates for a new task?
  - Given: Task description requiring TDD workflow
  - Expected: Agent defines precheck and postcheck gates
  - Success criteria: Gates match intended workflow, use appropriate checkers

- **Gate Response**: Can agent interpret gate failures and fix issues?
  - Given: Issue with failing `clippy` gate
  - Expected: Agent reads gate failure, identifies issue, fixes code
  - Success criteria: Agent resolves issue without human intervention

- **Gate Preview**: Can agent use `jit gate preview` effectively for planning?
  - Given: Issue with prechecks defined
  - Expected: Agent runs preview before claiming issue
  - Success criteria: Agent adapts workflow based on precheck results

- **Gate Customization**: Can agent modify gate definitions appropriately?
  - Given: Gate that needs timeout adjustment or different checker
  - Expected: Agent updates gate definition
  - Success criteria: Updated gate works as intended

### 2. MCP Server Tool Coverage

**Objective**: Verify MCP tools provide complete functionality without CLI fallback.

**Test Scenarios**:

- **Complete Workflow**: Agent completes full issue lifecycle via MCP only
  - Create issue → Add gates → Claim → Work → Pass gates → Complete
  - No CLI commands needed
  - All operations through MCP tools

- **Tool Discovery**: Agent can find appropriate tools for tasks
  - Given: Natural language task (e.g., "find all high priority issues")
  - Expected: Agent selects correct MCP tool
  - Success criteria: Efficient tool selection without trial-and-error

- **Error Handling**: Agent handles MCP tool errors gracefully
  - Given: Tool failure (e.g., invalid issue ID)
  - Expected: Agent parses error, takes corrective action
  - Success criteria: Agent recovers without user intervention

- **Complex Queries**: Agent uses advanced query tools effectively
  - Strategic queries, label wildcards, blocked issue analysis
  - Combines multiple queries for insights
  - Generates actionable reports

### 3. Documentation Adequacy

**Objective**: Verify agent can onboard and become productive using only documentation.

**Test Scenarios**:

- **Cold Start**: Fresh agent (no prior context) reads docs and completes task
  - Given: AGENT-QUICKSTART.md only
  - Task: Create task with gates, complete TDD workflow
  - Success criteria: Agent completes task with <3 documentation lookups

- **Gate Examples**: Agent can adapt gate examples to new scenarios
  - Given: docs/gate-examples.md
  - Task: Create gates for Go project (examples show Rust/Python/JS)
  - Success criteria: Agent creates working Go gates

- **Troubleshooting**: Agent can resolve common issues using documentation
  - Given: Error messages and docs
  - Task: Fix validation errors, gate failures, dependency cycles
  - Success criteria: Agent resolves issues without human help

### 4. Workflow Efficiency

**Objective**: Measure token efficiency and agent interaction patterns.

**Metrics**:

- **Token Usage**: Measure tokens consumed for common workflows
  - Baseline: Complete TDD cycle (create issue → implement → test → gate → done)
  - Target: <20k tokens for typical task
  - Track: JSON output reduces token waste vs. parsing text

- **Tool Call Efficiency**: Count tool calls needed per workflow
  - Baseline: Claim issue → Check status → Run gates → Complete
  - Target: <10 tool calls for typical workflow
  - Track: Unnecessary polling or redundant calls

- **Error Recovery Time**: Measure agent response to failures
  - Baseline: Gate failure → Fix → Re-run
  - Target: <5 tool calls to recover
  - Track: Effective error messages reduce trial-and-error

### 5. Multi-Agent Coordination

**Objective**: Verify multiple agents can work concurrently without conflicts.

**Test Scenarios**:

- **Concurrent Claims**: Multiple agents claim different ready issues
  - Expected: Atomic claim operations, no double-assignment
  - Success criteria: No conflicts, fair distribution

- **Dependency Coordination**: Agent A completes issue, unblocking Agent B
  - Expected: Agent B detects unblocking and proceeds
  - Success criteria: Minimal latency, correct state transitions

- **Conflicting Updates**: Multiple agents modify different issues with shared dependencies
  - Expected: File locking prevents corruption
  - Success criteria: All updates succeed, DAG maintained

## Test Methodology

### Phase 1: Controlled Scenarios (Week 1)

**Setup**:
- Use Claude (via Anthropic API) and GitHub Copilot (via CLI)
- Prepare test repository with realistic issue structure
- Define 10 standard workflows (TDD cycle, gate management, queries, etc.)

**Execution**:
- Run each workflow 3 times with fresh agent context
- Record: Tool calls, token usage, success rate, failure modes
- Collect agent feedback (via prompting for self-assessment)

**Analysis**:
- Identify common failure patterns
- Measure against efficiency targets
- Prioritize issues blocking common workflows

### Phase 2: Production Simulation (Week 2)

**Setup**:
- Larger repository (100+ issues)
- Multiple agents working concurrently (2-4 agents)
- Realistic dependencies and gate configurations

**Execution**:
- Agents complete sprint: claim issues, implement features, pass gates
- Run for 1-2 days of simulated work
- Minimal human intervention (only for blocking bugs)

**Analysis**:
- Multi-agent coordination issues
- Scalability bottlenecks
- Documentation gaps
- UX friction points

### Phase 3: Iteration & Fix (Week 3)

**Execution**:
- Fix identified issues (prioritize blocking bugs)
- Update documentation based on agent confusion patterns
- Add missing MCP tools or CLI commands
- Re-run validation scenarios

**Success Criteria**:
- 90% workflow success rate (agent completes without human help)
- <20k tokens per typical workflow
- <10 tool calls per typical workflow
- Zero data corruption issues

## Validation Environment

### Test Repository Structure

```
.jit/
  data/
    issues/
      [100 test issues: mix of milestones, epics, tasks]
  config/
    gates.json [Pre-configured gates: TDD, security, etc.]

docs/
  [Test documents for knowledge management]

src/
  [Dummy code for gate execution]
```

### Agent Configuration

**Claude**:
- Model: claude-3-7-sonnet-20250219 (latest)
- System prompt: Include AGENT-QUICKSTART.md
- Tools: All 47 MCP tools enabled

**GitHub Copilot**:
- CLI mode with MCP integration
- Standard workspace configuration

### Metrics Collection

- **Automated**: JSON logs of all tool calls, timestamps, outcomes
- **Manual**: Observer notes on agent behavior patterns
- **Agent self-report**: Prompt agent to rate difficulty and suggest improvements

## Expected Outcomes

### Bugs to Fix

Anticipated categories:
- Missing MCP tools for specific operations
- Confusing error messages that agents can't parse
- Documentation gaps (missing examples, unclear concepts)
- Performance issues with large result sets
- Race conditions in concurrent scenarios

### Documentation Updates

- Add agent-specific troubleshooting section
- Expand gate examples with more languages/scenarios
- Create "common pitfalls" guide for agents
- Add workflow diagrams for visual learners
- Improve JSON schema documentation

### Feature Additions

Likely requirements:
- Additional query filters
- Better gate failure diagnostics
- Batch operations for efficiency
- Enhanced event querying for coordination

## Success Metrics

**Production Readiness Criteria**:

1. **Workflow Success Rate**: ≥90% of workflows complete without human intervention
2. **Token Efficiency**: ≤20k tokens for standard TDD workflow
3. **Tool Call Efficiency**: ≤10 tool calls for standard workflow
4. **Error Recovery**: ≥80% of errors resolved by agent without help
5. **Multi-Agent Safety**: Zero data corruption in concurrent tests
6. **Documentation Coverage**: Agent can complete all standard workflows using docs only

**Minimum Viable v1.0**:
- All 6 criteria met
- No blocking bugs discovered
- Documentation complete and validated

## Implementation Plan

**Week 1**: Controlled scenario testing
**Week 2**: Production simulation
**Week 3**: Bug fixes and iteration
**Week 4**: Final validation and sign-off

**Effort**: 3-4 weeks with 1-2 developers + AI agent access

## Dependencies

- Phase 5.2 complete (gate preview, history, enhanced errors)
- Documentation epic complete (EXAMPLE.md, user guide, agent quickstart)
- MCP server stable and deployed

## Deliverables

1. **Validation Report**: Detailed results from all test phases
2. **Bug List**: Prioritized issues discovered during validation
3. **Documentation Updates**: All gaps filled based on agent feedback
4. **Benchmark Results**: Token usage, tool call counts, success rates
5. **Production Readiness Sign-off**: Go/no-go recommendation for v1.0
