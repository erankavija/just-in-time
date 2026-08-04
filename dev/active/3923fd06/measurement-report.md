# Measurement Report — Published Package Contribution Drift

> **Diátaxis Type:** Reference

This report records the contribution comparison for issue `3923fd06`. The
comparison binds each contribution from every package under `profiles/` to the
registry entry it restates.

## Measured Set

The published package set is `jit-default` and `jit-dogfood`, derived from the
two package-source directories under `profiles/`. The comparison mechanism is
`contribution_drift_reports` in
`crates/jit/src/profile/contribution_drift.rs`; its shared report shape is
`crates/jit/src/profile/drift_report.rs`. The registry inputs are
`.jit/config.toml`, `.jit/invariants.toml`, `.jit/rules.toml`, and
`.jit/gates.toml`.

The repaired comparison contains six `jit-default` namespace contributions and
eleven field-level differences. Every difference is recorded below. The
package-side repairs are the `type` and `resolution` example-list additions;
the other repairs change the repository registry carrier.

## Difference Register

The package and repository texts in this table are the carrier values supplied
to the comparison at the parent of `b80fe588`. Array values show the complete
`examples` list so membership and ordering are measurable.

| Subject | Package and packaged text | Repository and repository text | Disposition |
| --- | --- | --- | --- |
| `namespaces.type` map entry; `jit-default` contribution at `profiles/jit-default/manifest.toml:112-121` | `description = "Issue type (hierarchical). At most one per issue."`; `examples = ["type:task", "type:story", "type:epic"]` | `.jit/config.toml:90-93`: `description = "Issue type (hierarchical). Exactly one per issue."`; `examples = ["type:task", "type:story", "type:epic", "type:milestone"]` | Repaired both carriers: `.jit/config.toml` adopts the packaged description, and `profiles/jit-default/manifest.toml` adds `"type:milestone"` to its package list. |
| `namespaces.milestone` map entry; `jit-default` contribution at `profiles/jit-default/manifest.toml:151-159` | `description = "Release milestone membership (version tag)."`; `examples = ["milestone:v1.0", "milestone:v1.2.3", "milestone:v2.0-rc1"]` | `.jit/config.toml:100-103`: `description = "Release or time-bounded goal membership"`; `examples = ["milestone:v1.0", "milestone:2026-q1"]` | Repaired the repository carrier: `.jit/config.toml` adopts the packaged description and example list. |
| `namespaces.component` map entry; `jit-default` contribution at `profiles/jit-default/manifest.toml:123-131` | `description = "Technical area or subsystem affected."`; `unique = false`; `examples = ["component:backend", "component:frontend", "component:cli"]` | `.jit/config.toml:110-113`: `description = "Technical area or subsystem"`; `unique = false`; `examples = ["component:backend", "component:frontend", "component:cli"]` | Repaired the repository carrier: `.jit/config.toml` adopts the packaged description. |
| `namespaces.team` map entry; `jit-default` contribution at `profiles/jit-default/manifest.toml:142-149` | `description = "Owning team."`; `unique = true`; `examples = ["team:backend", "team:platform"]` | `.jit/config.toml:115-118`: `description = "Owning team"`; `unique = true`; `examples = ["team:core", "team:platform"]` | Repaired the repository carrier: `.jit/config.toml` adopts the packaged description and example list. |
| `namespaces.resolution` map entry; `jit-default` contribution at `profiles/jit-default/manifest.toml:161-169` | `description = "Reason for issue closure (used with rejected state)."`; `examples = ["resolution:wont-fix", "resolution:duplicate"]` | `.jit/config.toml:120-123`: `description = "Reason for issue closure"`; `examples = ["resolution:wont-fix", "resolution:duplicate", "resolution:obsolete"]` | Repaired both carriers: `.jit/config.toml` adopts the packaged description, and `profiles/jit-default/manifest.toml` adds `"resolution:obsolete"` to its package list. |
| `namespaces.enforces` map entry; `jit-default` contribution at `profiles/jit-default/manifest.toml:171-179` | `description = "Enforcement link: names an invariant, rule, or gate item that the labeled issue enforces."`; `unique = false`; `examples = ["enforces:@/invariant/label-format", "enforces:@/rule/label-format", "enforces:@/gate/cargo-ci"]` | `.jit/config.toml:135-138`: `description = "Enforcement link: names an invariant, rule, or gate item that the labeled issue enforces"`; `unique = false`; `examples = ["enforces:@/invariant/label-format", "enforces:@/rule/label-format", "enforces:@/gate/cargo-ci"]` | Repaired the repository carrier: `.jit/config.toml` adopts the packaged description. |

The six entry rows expand to eleven field-level differences: two in `type`,
three in `milestone`, one in `component`, two in `team`, two in `resolution`,
and one in `enforces`. The changed package lists are the `type` and
`resolution` lists; their added members are present in the final registry
entries at `.jit/config.toml:93` and `.jit/config.toml:123`.

## Override Status

No difference in the register is declared as an override. The existing
package-scoped declarations in
`crates/jit/src/profile/repository_package.rs:1026-1100` include the
`jit-dogfood` namespace-example override. Its declared reason is:

> Namespace examples are documentation and never enforced, so each carrier names its own vocabulary: this repository's examples cite its own issues, which an adopter has no counterpart for.

That override is scoped to `jit-dogfood`; it does not cover any `jit-default`
contribution in the register.

## Clean Comparison Result

`test_contribution_drift_is_empty_across_every_published_package` runs
`published_contribution_drift` over both discovered packages and returns an
empty report list. `test_every_published_package_contributes_to_a_registry_the_walk_reads`
also passes, confirming that both published packages enter the comparison and
each contributes to a registry it reads.

The comparison result is therefore zero remaining drift reports over the
published package set.

`docs/reference/example-config.toml` is not a comparison carrier: no profile
contribution declares it as a registry target. Its `team` example change in
`7b013c12` is outside the six contribution reports and is not counted here.

## Evidence

- `b80fe588` supplies the package-dimension comparison and the registry and
  manifest repairs.
- `7c892dd3` formats the package-dimension implementation.
- `7b013c12` changes the unbound example configuration document.
- `.jit/config.toml:61-73` declares the type hierarchy and its label
  associations; `.jit/config.toml:90-138` declares the namespace registry.
- `.jit/rules.toml` derives `namespace-unique-*` rules from declared namespaces;
  the `type` rule states `min = 0` and `max = 1`.
- The repository registries `.jit/invariants.toml`, `.jit/rules.toml`, and
  `.jit/gates.toml` were read as comparison context and were not edited.
