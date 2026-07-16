# Repository Dogfood Content Standards

> **Diátaxis Type:** Reference

This is a stable redirect for contributors to the JIT source repository. Its
canonical, profile-managed content standard lives at
[`.jit/reference/content-standards.md`](../../.jit/reference/content-standards.md).

The embedded `jit-dogfood` package carries that asset for its workflow skills
and review prompts. Ordinary `jit init` does not install it;
`jit init --profile jit-dogfood` and
`jit profile apply jit-dogfood` do. See
[Repository Profiles](profiles.md) for the package and lifecycle contract.
The content standard is profile-installed workflow policy, not an engine
default or a requirement for repositories that use plain initialization.
