# 70ac98fc: cargo-ci evidence and commit ancestry

Facts for the code-review gate of issue 70ac98fc. All times are UTC.

| Fact | Value |
|---|---|
| Attributable commit | `058434a44612d012a28c013cb6ae2ff748a49f14`, authored and committed 2026-11-03T22:24:36Z |
| Merge into main | `b383182558534d10bc04943579eba5d0728b2dba`, committed 2026-11-03T22:27:08Z; first parent `078a993dd`, second parent `845b758db` (branch tip, which contains `058434a44`) |
| cargo-ci gate run | `fca270bd-e48e-486f-83c7-ec5ba54d2919`, commit `b383182558534d10bc04943579eba5d0728b2dba`, started 2026-11-03T22:27:08Z, completed 2026-11-03T22:28:14Z, passed: 4,811 tests passed, 0 failed, 0 skipped; 25 doctests passed, 0 ignored |
| Prior code-review run | `bb4fa0cd-010d-4ee4-9cfa-b1771245aa7b`, commit `b383182558534d10bc04943579eba5d0728b2dba`, started 2026-11-03T22:28:14Z, failed |

Ancestry check, run in the repository root:

```
$ git merge-base --is-ancestor 058434a44 b38318255; echo "exit=$?"
exit=0
$ git merge-base --is-ancestor b38318255 058434a44; echo "exit=$?"
exit=1
```
