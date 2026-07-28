# JIT Product Documentation

Welcome to **Just-In-Time (JIT)** - a CLI-first issue tracker designed for programmatic agents with dependency graphs and quality gates.

## Documentation Structure

This documentation follows the [Diátaxis](https://diataxis.fr/) framework:

### 📚 [Concepts](concepts/) - Understanding JIT
*Explanation-oriented: Learn about core concepts and design principles*

- [Overview](concepts/overview.md) - What is JIT and why it exists
- [Core Model](concepts/core-model.md) - Issues, dependencies, gates, states
- [Containment and Completion](concepts/containment-and-completion.md) - One edge kind, derived containment, why containers finish last
- [Hierarchy Resolution](concepts/hierarchy-resolution.md) - Why the dependency DAG, not labels, defines containment
- [Scope](concepts/scope.md) - Domain coverage and boundaries
- [System Guarantees](concepts/guarantees.md) - Invariants and consistency
- [Design Philosophy](concepts/design-philosophy.md) - Domain-agnostic principles
- [Methodology-Agnostic Validation](concepts/validation-engine.md) - Why validation is configuration, not code
- [The Plan-Before-Fan-Out Bracket](concepts/planning-bracket.md) - Gated planning sequenced before the implementation fan-out

### 🎓 [Tutorials](tutorials/) - Learning-Oriented
*Step-by-step lessons to get started*

- [Quickstart](tutorials/quickstart.md) - Get started in 10 minutes
- [First Workflow](tutorials/first-workflow.md) - Complete walkthrough with epic → tasks workflow
- [Parallel Work with Git Worktrees](tutorials/parallel-work-worktrees.md) - Multi-agent coordination

### 🔧 [How-To Guides](how-to/) - Goal-Oriented
*Practical recipes for specific use cases*

- [Software Development](how-to/software-development.md) - Feature dev, TDD, CI/CD
- [Research Projects](how-to/research-projects.md) - Research questions, experiments
- [Knowledge Work](how-to/knowledge-work.md) - Personal projects, learning goals
- [Validation Rules](how-to/validation-rules.md) - Author `.jit/rules.toml` rules and schemas
- [Manually Adopt the Planning Bracket](how-to/adopt-planning-bracket.md) - Advanced configuration for gated planning before the fan-out
- [Custom Gates](how-to/custom-gates.md) - Define and use quality gates
- [Dependency Management](how-to/dependency-management.md) - Graph strategies
- [Multi-Agent Coordination](how-to/multi-agent-coordination.md) - Team and parallel work
- [Deployment](how-to/deployment.md) - Running the web UI
- [Troubleshooting](how-to/troubleshooting.md) - Common issues and solutions

### 📖 [Reference](reference/) - Information-Oriented
*Technical specifications and API documentation*

- [CLI Commands](reference/cli-commands.md) - Complete command reference
- [Repository Profiles](reference/profiles.md) - Preferred embedded workflow setup, package contract, and recovery boundary
- [Exit Codes](reference/exit-codes.md) - Process exit-code taxonomy and per-command mappings
- [Machine-readable Error Codes](reference/error-codes.md) - Generated vocabulary of error-envelope codes, meanings, and exit statuses
- [CLI Command-Grammar Standard](reference/cli-command-grammar.md) - Canonical command grammar (nouns/verbs, positionals, id acceptance, gate grouping)
- [Storage Format](reference/storage-format.md) - On-disk format specification
- [Storage Record Layout](reference/storage-records.md) - Generated projection of issue identifiers, event-log serialization, and the gate-run record
- [Event Log Tags](reference/events.md) - Generated catalog of event tags, scopes, and `issue_id` presence
- [Configuration](reference/configuration.md) - config.toml and settings
- [Runtime Coordination Defaults](reference/runtime-defaults.md) - Built-in lock, cleanup, and claim-TTL defaults
- [Item Addresses](reference/item-addresses.md) - Address grammar for addressable structured items
- [Glossary](reference/glossary.md) - Term definitions
- [Claim System](reference/claim.md) - Leases and coordination
- [Example Config](reference/example-config.toml) - Sample configuration
- [Labels](reference/labels.md) - Label system reference
- [Rules and Gates](reference/rules-and-gates.md) - Projected reference for a project's validation rules and gate registry
- [Built-in Gate Presets](reference/gate-presets.md) - The gate bundles the binary ships, with each preset's gates and checkers
- [Worktree and Validate Commands](reference/worktree-validate.md) - `jit worktree` and `jit validate` command reference

### 🧪 [Examples](examples/) - Sample Configurations and Rulesets
*Ready-to-copy configuration and ruleset examples for common domains, referenced throughout [Validation Rules](how-to/validation-rules.md)*

These are advanced customization examples. For the portable recommended
workflow, start with the
[embedded `jit-dogfood` profile](reference/profiles.md).

- [sdd](examples/sdd/) - Spec-Driven Development
- [bug-repro](examples/bug-repro/) - bug triage
- [release-checklist](examples/release-checklist/) - release gating
- [fresh-evidence](examples/fresh-evidence/) - fresh-evidence-before-done
- [nyquist](examples/nyquist/) - criteria-to-check mapping
- [cross-epic](examples/cross-epic/) - cross-epic requirement-id collision detection
- [research](examples/research/) - research program: non-software hierarchy with `type:goal` / `type:experiment` and `hyp:` / `tests:` namespaces

### 📄 Additional Resources

- [Main README](../README.md) - Project overview and quick links
- [Development Documentation](../dev/index.md) - For contributors working on JIT itself

---

## Getting Started

1. **New to JIT?** Start with [Concepts](concepts/) to understand the core model
2. **Want to try it?** Follow the [Tutorials](tutorials/)
3. **Solving a specific problem?** Check [How-To Guides](how-to/)
4. **Need technical details?** See [Reference](reference/)

## About This Documentation

**Product documentation** (`docs/`) is permanent, user-facing reference material that:
- Uses domain-agnostic terminology (works for software, research, knowledge work)
- Stays stable across releases
- Never gets archived

**Development documentation** (`dev/`) covers how we build JIT itself - see [dev/index.md](../dev/index.md) for contributor resources.

---
