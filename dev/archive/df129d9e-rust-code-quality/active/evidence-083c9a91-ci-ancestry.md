# 083c9a91: cargo-ci evidence and commit ancestry

Facts for the code-review gate of issue 083c9a91. All times are UTC.

| Fact | Value |
|---|---|
| Attributable commit | `2174da3a86197ff5a3617651a6fcab760530715b`, authored and committed 2026-11-02T23:35:24Z (recorded as 01:35:24 +02:00) |
| Merge into main | `0fbd890c3f44e1df9f0a03a531d6abd113cb7426`, committed 2026-11-03T01:09:33Z; second parent `acd682e3f` |
| cargo-ci gate run | `44b22429-4611-4d33-b08c-12caff2d8b9c`, commit `0fbd890c3f44e1df9f0a03a531d6abd113cb7426`, started 2026-11-03T01:09:34Z, completed 2026-11-03T01:11:12Z, passed |
| Prior code-review run | `ee657077-57ea-4ce0-944b-48811c667ac2`, commit `0fbd890c3f44e1df9f0a03a531d6abd113cb7426`, started 2026-11-03T01:11:13Z, failed |

Ancestry check, run in the repository root:

```
$ git merge-base --is-ancestor 2174da3a 0fbd890c3; echo "exit=$?"
exit=0
```
