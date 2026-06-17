# JIT Product Documentation

Welcome to **Just-In-Time (JIT)** - a CLI-first issue tracker designed for programmatic agents with dependency graphs and quality gates.

## Documentation Structure

This documentation follows the [Diátaxis](https://diataxis.fr/) framework:

### 📚 [Concepts](concepts/) - Understanding JIT
*Explanation-oriented: Learn about core concepts and design principles*

- [Overview](concepts/overview.md) - What is JIT and why it exists
- [Core Model](concepts/core-model.md) - Issues, dependencies, gates, states
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
- [Adopt the Planning Bracket](how-to/adopt-planning-bracket.md) - Wire gated planning before the fan-out
- [Custom Gates](how-to/custom-gates.md) - Define and use quality gates
- [Dependency Management](how-to/dependency-management.md) - Graph strategies
- [Multi-Agent Coordination](how-to/multi-agent-coordination.md) - Team and parallel work
- [Deployment](how-to/deployment.md) - Running the web UI
- [Troubleshooting](how-to/troubleshooting.md) - Common issues and solutions

### 📖 [Reference](reference/) - Information-Oriented
*Technical specifications and API documentation*

- [CLI Commands](reference/cli-commands.md) - Complete command reference
- [Storage Format](reference/storage-format.md) - On-disk format specification
- [Configuration](reference/configuration.md) - config.toml and settings
- [Glossary](reference/glossary.md) - Term definitions
- [Claim System](reference/claim.md) - Leases and coordination
- [Example Config](reference/example-config.toml) - Sample configuration
- [Labels](reference/labels.md) - Label system reference

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


