## Project Invariants

<!-- jit:invariants:begin -->
_No invariants declared._
<!-- jit:invariants:end -->

## JIT workflow

- Treat `.jit/` as repository-owned workflow configuration and issue data.
- Read `.jit/reference/content-standards.md` before authoring issues or planning documents.
- Derive hierarchy, templates, gates, namespaces, and documentation paths from repository configuration.
- A label means what its `[namespaces.<ns>]` declaration in `.jit/config.toml` says it means; read that declaration before judging what a label on an issue claims.
- Use `jit issue status`, `jit query available`, and the dependency graph to select and sequence work.
- Run the configured gates before completing an issue; a passing review placeholder is advisory evidence only.
