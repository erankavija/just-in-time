# df129d9e completion report

Production lines of `crates/jit/src` and the build-footprint budgets, measured at a named final commit against the [baseline](line-count-baseline.md).

## Commits

| Role | Commit |
|---|---|
| Baseline | `dc2ea8706edd285b09746ec7f111ce405845e961` |
| Final | `ce7da387c72e5b36c2f5e1dc2fc982213e6d1e52` |

The counter is `scripts/production-lines.py`; its counting logic is unchanged since the baseline (`git diff 9217c5821 ce7da387c -- scripts/production-lines.py` only adds `--file` and argparse), and the baseline total reproduces at 78801. Every figure below is also in [ledger-final.txt](ledger-final.txt), the output of:

```sh
python3 dev/active/df129d9e-rust-code-quality/ledger-final.py \
  dc2ea8706edd285b09746ec7f111ce405845e961 ce7da387c72e5b36c2f5e1dc2fc982213e6d1e52
```

The script reads git and the live tracker (`jit issue list --full --json`, `jit graph deps df129d9e --depth 0 --json`); it runs in about 24 minutes.

## Production lines per module (REQ-01)

Commands, as in the baseline:

```sh
python3 scripts/production-lines.py <rev>                                                # default
python3 scripts/production-lines.py <rev> --strip-line-comments --test-support-as-test   # flagged
```

Default is the REQ-10 evidence. A dash marks a module absent at that commit or excluded by the mode. Source: the `module` rows of `ledger-final.txt`.

| Module | Default base | Default final | Default Δ | Flagged base | Flagged final | Flagged Δ |
|---|---:|---:|---:|---:|---:|---:|
| `commands` | 21113 | 19159 | -1954 | 19926 | 18258 | -1668 |
| `storage` | 9727 | 11593 | +1866 | 9313 | 10715 | +1402 |
| `repository_state` | 10620 | 9367 | -1253 | 10325 | 9168 | -1157 |
| `domain` | 7483 | 8258 | +775 | 7347 | 7982 | +635 |
| `main` | 7965 | 37 | -7928 | 7366 | 34 | -7332 |
| `profile` | 5041 | 3123 | -1918 | 4370 | 2975 | -1395 |
| `output` | 1839 | 5014 | +3175 | 1782 | 4834 | +3052 |
| `dispatch` | — | 3661 | +3661 | — | 3632 | +3632 |
| `validation` | 2952 | 3076 | +124 | 2763 | 2916 | +153 |
| `cli` | 1672 | 1375 | -297 | 1661 | 1366 | -295 |
| `declarations` | 1369 | 1668 | +299 | 1303 | 1620 | +317 |
| `document` | 1237 | 1185 | -52 | 1174 | 1126 | -48 |
| `config` | 1149 | 704 | -445 | 1105 | 695 | -410 |
| `test_utils` | 1029 | 1141 | +112 | — | — | — |
| `schema` | 1013 | 600 | -413 | 912 | 514 | -398 |
| `graph` | 831 | 853 | +22 | 781 | 803 | +22 |
| `errors` | 734 | 760 | +26 | 728 | 754 | +26 |
| `test_harness` | — | 605 | +605 | — | — | — |
| `templates` | 366 | 591 | +225 | 353 | 581 | +228 |
| `gate_execution` | 457 | 384 | -73 | 406 | 342 | -64 |
| `gate_presets` | 444 | 341 | -103 | 421 | 199 | -222 |
| `query_engine` | 375 | 387 | +12 | 366 | 378 | +12 |
| `test_cli` | — | 287 | +287 | — | — | — |
| `generated_artifacts` | 148 | 204 | +56 | — | — | — |
| `search` | 140 | 197 | +57 | 129 | 183 | +54 |
| `test_taxonomy` | 151 | 188 | +37 | — | — | — |
| `visualization` | 177 | 188 | +11 | 172 | 183 | +11 |
| `config_manager` | 149 | 72 | -77 | 141 | 68 | -73 |
| `snapshot` | 142 | 112 | -30 | 141 | 112 | -29 |
| `labels` | 112 | 136 | +24 | 100 | 127 | +27 |
| `runtime_defaults` | 62 | 120 | +58 | 53 | 8 | -45 |
| `agent_config` | 88 | 93 | +5 | 79 | 86 | +7 |
| `output_macros` | 71 | — | -71 | 68 | — | -68 |
| `type_icons` | 46 | 62 | +16 | 42 | 58 | +16 |
| `lib` | 49 | 47 | -2 | 42 | 36 | -6 |
| `test_lease_store` | — | 46 | +46 | — | — | — |
| `process` | — | 33 | +33 | — | 33 | +33 |
| `build_info` | 23 | 23 | +0 | 23 | 23 | +0 |
| `test_mode` | 15 | — | -15 | 3 | — | -3 |
| `tar_format` | 12 | 12 | +0 | 12 | 12 | +0 |
| **TOTAL** | 78801 | 75702 | -3099 | 73407 | 69821 | -3586 |

## Build-footprint budgets (REQ-02)

| Measurement | `rust-build-budget:` line |
|---|---|
| Before the harness change | `integration-targets=10/12 active-executables=14 bytes=1743471112/2147483648` |
| Final commit | `integration-targets=3/3 active-executables=7 bytes=644461600/740294656 profile=line-tables-only dep-features=pruned dynamic-dependencies=checked main-rs-lines=37/40` |

The final line was measured in a worktree at `ce7da387c`, with the steps `scripts/cargo-ci.sh` runs before its budget step (exit 0):

```sh
CARGO_INCREMENTAL=0 cargo build --workspace --bins
CARGO_INCREMENTAL=0 cargo test --workspace --no-run --message-format=json > artifacts.json
./scripts/rust-build-budget.sh --root "$PWD" --artifacts-json artifacts.json
```

## Attribution of the largest module deltas (REQ-03)

These are the five modules with the largest absolute default delta. The class sums come from the `attr-sum` rows of `ledger-final.txt`, and the per-merge module deltas from its `attr` rows. A class sum adds the module's delta over every first-parent step in that class. `epic` means a df129d9e merge, `other` a merge of another epic's issue, and `direct` a non-merge commit.

| Module | Δ | epic | ambiguous | other | direct | Largest contributing merges (module Δ) |
|---|---:|---:|---:|---:|---:|---|
| `main` | -7928 | -7615 | +0 | -313 | +0 | `c5a125d74` w7batch (14 df129d9e issues) -2080; `860d8207e` 1ac651c7 -701; `b319c484a` 3f816619 -592; `b7ebfe569` f87f9b00 -560 |
| `dispatch` | +3661 | +3527 | +15 | +119 | +0 | `c5a125d74` w7batch +1287; `b319c484a` 3f816619 +368; `860d8207e` 1ac651c7 +358; `f0fc5542b` 10751360 +245 |
| `output` | +3175 | +3263 | +20 | -108 | +0 | `c5a125d74` w7batch +1056; `e20c3b743` 1874e265 +432; `70457e502` fb38bf1f -406; `860d8207e` 1ac651c7 +319 |
| `commands` | -1954 | +1099 | +0 | -3047 | -6 | `77cfcd1f4` 24268b05 (epic:lead-friction) -433; `70457e502` fb38bf1f (df129d9e) +426; `7f4b74e92` 866bdde0 (epic:greenfield-cutover) -317; `f0beca0be` abe963dd (epic:lead-friction) -274 |
| `profile` | -1918 | -8 | +0 | -1910 | +0 | `7abf2f2ff` e7d98542 (epic:test-topology) -519; `77cfcd1f4` 24268b05 (epic:lead-friction) -283; `07c2bdd2c` 2dd6b833 (epic:lead-friction) -282; `a9e1ac051` 216f70b4 and four more (epic:lead-friction) -266 |

The `main`, `dispatch` and `output` deltas are this epic's handler moves out of `main.rs`. The `commands` and `profile` reductions come from concurrent epics, mainly `epic:lead-friction`, `epic:greenfield-cutover` and `epic:test-topology`.

## Ledger of df129d9e merges (REQ-04)

Each delta is `TOTAL(merge) − TOTAL(merge^1)` in default mode, for every first-parent merge on `main` in `dc2ea8706..ce7da387c` whose issue ids are all labelled `epic:rust-code-quality`. A merge's ids come from its branch name (`worktree-agent-<id>`) and its `jit:<id>` tokens, or otherwise from the merged side's commit messages. Source: the `step epic` rows of `ledger-final.txt`.

| Sum | Merges | Default Δ | Ceiling (epic REQ-10, plan D-54) |
|---|---:|---:|---:|
| df129d9e merges | 125 | **+306** | +375 |
| df129d9e merges plus the ambiguous merge below | 126 | +344 | +375 |

Crate total at the final commit: **75702** (baseline 78801, Δ −3099).

Reconciliation, from the `sum` rows: epic +306, ambiguous +38, other epics' merges −3458 (557 merges), direct commits +15 (2273 commits, of which only `022a46af6`, c780514e (epic:test-topology), at +10 and `96dfde725`, 78d897a2 (epic:one-validation-report), at +5 touch the count). The parts add up to −3099.

Cross-checks: all 76 merges in `ledger-2026-11-04.txt` and all 92 in `ledger-2026-11-05.txt` appear here with identical deltas. The running sum is +171 after 101 merges at `f004e7cd3`, matching `handoff-12.md`, and +184 after `49c2582ea`, matching `handoff-14.md`. The merges after that are 0b35be66 at `ae01461e7` (+122), afd2d573 at `bed5d7f2a`, 7909527f at `2318609d5`, f487aeef at `48c408716` and `c24f83489`, and fba8d478 at `ba3d2b076` and `38670c86b`. All of them except 0b35be66 have a delta of 0.

### Excluded or ambiguous

| Merge | Issue | Δ | Reason |
|---|---|---:|---|
| `0d980ee38` | dc048377 | +38 | It has no `epic:` label, but it is in df129d9e's transitive dependency tree as a prerequisite of 0546fc82, which is in this epic. It is counted in neither sum above. |

Merges whose ids belong only to other epics' labels, or to unlabelled issues outside df129d9e's dependency tree, are excluded. They are the `step other` rows of `ledger-final.txt`, with their epic labels.

### Per-merge deltas

| Class | Merge | Issues | Δ |
|---|---|---|---:|
| epic | `e84318f92` | 9ca31c64 | -2 |
| epic | `e20c3b743` | 1874e265 | +11 |
| epic | `6470711b4` | ef9897ab | +258 |
| epic | `40aaec181` | 719a5501 | -121 |
| epic | `6d24ab6ee` | f7f01654 | +129 |
| epic | `4a9f6f686` | 233f4a09 | +0 |
| epic | `988b20792` | 650e6a56 | -94 |
| epic | `fae1d1a07` | 650e6a56 | +0 |
| epic | `f767033dc` | d8176fd3 | -318 |
| epic | `78975ecf1` | d8176fd3 | +0 |
| epic | `d144d3d84` | 27f0a575 | +47 |
| epic | `be0d9ab53` | 27f0a575 | +0 |
| epic | `b1df0b2f4` | 2f15d48d | +55 |
| epic | `12d718456` | 9fa12e75 | +28 |
| epic | `ce437049d` | 9fa12e75 | +18 |
| epic | `145091232` | 9cc3056a | +14 |
| epic | `8cf19175a` | d4373d07 | +40 |
| epic | `8c6d518bc` | d4373d07 | +0 |
| epic | `10fde232e` | 2e7e1348 | -4 |
| epic | `853d1cfc8` | a807e699 | +12 |
| epic | `b834fcbae` | a807e699 | +11 |
| epic | `46a747fc2` | 5d0a9a90 | +8 |
| epic | `1d998db27` | 5d0a9a90 | +0 |
| epic | `99b8ae0dd` | d84a7bfa | +18 |
| epic | `55e6ce46f` | 1002524a | +14 |
| epic | `0fbd890c3` | 083c9a91 | +12 |
| epic | `18a5a57b0` | a015dcbb | +16 |
| epic | `d7729e1ca` | 6f49f3b3 | +52 |
| epic | `6f110c219` | f79d2510 | +17 |
| epic | `876eae0af` | 7a7d0bfe | +19 |
| epic | `3aa8182c5` | 985c2b15 | +33 |
| epic | `58b9b90dc` | fd2073b7 | +34 |
| epic | `226db21cc` | 93aaba24 | +21 |
| epic | `8b4ca7956` | dbd05242 | +11 |
| epic | `7f257f027` | 5962427b | +36 |
| epic | `67df5d08a` | a24b11aa | +10 |
| epic | `fbbaa2ba4` | 334c12e7 | +16 |
| epic | `92858ba2d` | d05b5639 | +33 |
| epic | `2f76a56e7` | a3a99cf8 | -131 |
| epic | `d28cc53a8` | 8b7f2973 | +16 |
| epic | `5934d9658` | 1539da29 | +43 |
| epic | `788a891f3` | 1539da29 | +0 |
| epic | `d11ed6ed8` | 22189592 | -5 |
| epic | `968119b03` | abe555e9 | +133 |
| epic | `6c0549aba` | 2e0f276b | +70 |
| epic | `e106ff343` | abe555e9 | -5 |
| epic | `7663d7dc3` | b23faa59 | -7 |
| epic | `114986b73` | 1eb949a5 | +0 |
| epic | `ab2dced07` | b23faa59 | +7 |
| epic | `4ec8f06cc` | e5d618ee | +0 |
| epic | `d735276b5` | e5d618ee | +0 |
| epic | `4baff2cb3` | 12c0f3fb | +166 |
| epic | `59257f573` | 8a14e8e9 | +0 |
| epic | `1e2d11ebf` | 8a14e8e9 | +0 |
| epic | `d51cddd0a` | 8a14e8e9 | +0 |
| epic | `f2e1aefd6` | e5d618ee | +6 |
| epic | `a476a2d42` | 153c9851 | +33 |
| epic | `f1e434642` | 9529b9ac | +56 |
| epic | `27a9330b5` | 9529b9ac | +4 |
| epic | `7f785b006` | c9103fcf | -31 |
| epic | `f0ed97f2e` | b51291e4 | +0 |
| epic | `2283280fa` | b51291e4 | +0 |
| epic | `e03e459d1` | d952d057 | +0 |
| epic | `dbbb1cb8b` | b51291e4 | +0 |
| epic | `992072bb4` | d952d057 | +0 |
| epic | `c5a125d74` | 06ee9b67,0fe46027,1a4f2ebd,20fc3b92,3cf69ecf,52110fb4,66790c84,71f5296c,8c46fc85,8e91126b,8f297e60,98cc2097,b25c2e54,f930acf2 | +306 |
| epic | `a5e36ed48` | 06ee9b67 | +0 |
| epic | `f1aa105be` | 182f4c03 | +25 |
| epic | `f0fc5542b` | 10751360 | +3 |
| epic | `86c39dff8` | cd8e04cc | -15 |
| epic | `98a98a2d1` | cd8e04cc | +6 |
| epic | `b81fe9705` | 66790c84 | +0 |
| epic | `a4685218e` | 1a4f2ebd | +0 |
| epic | `860d8207e` | 1ac651c7 | -24 |
| epic | `a6ec3caa3` | cc4ea8a9 | -74 |
| epic | `cfd894128` | 1a4f2ebd | -5 |
| epic | `bd41c168f` | 4a01ccba | +12 |
| epic | `9182cf938` | 4a01ccba | +0 |
| epic | `a1e738ccb` | f82a2e4a | -60 |
| epic | `db15b3915` | 4a01ccba | +0 |
| epic | `86b4360d9` | 3b696589 | +0 |
| epic | `fc511300c` | 4a01ccba | +16 |
| epic | `baa3a76ba` | 10fdad38 | -221 |
| epic | `3afea694e` | 9be5b0de | -23 |
| epic | `3494915bf` | 3b696589 | +0 |
| epic | `b64962ffa` | 4a01ccba | +0 |
| epic | `0d17458ef` | 8fbf8c43 | -31 |
| epic | `af1d0b839` | 24a982b7 | -8 |
| epic | `b7ebfe569` | f87f9b00 | -114 |
| epic | `ae99aaa66` | 24a982b7 | +35 |
| epic | `9ea35b033` | f87f9b00 | +0 |
| epic | `b319c484a` | 3f816619 | -105 |
| epic | `93e9844b7` | b23b1bf3 | -53 |
| epic | `b8b8be033` | 5687e281 | -175 |
| epic | `b1a624617` | da30b92a | -8 |
| epic | `d43510d02` | ae3f4f8b | -13 |
| epic | `b8b4a0854` | 0bc448fb | +31 |
| epic | `87938c3d3` | 8b3cce62 | -106 |
| epic | `fad481d3e` | 6acd7a05 | +4 |
| epic | `e1b2bf52b` | 8b3cce62 | -21 |
| epic | `f004e7cd3` | 6acd7a05 | +0 |
| epic | `437c7ccfd` | f855d631 | +0 |
| epic | `acabe5453` | 5f12265a | +0 |
| epic | `61cdc4ec4` | 5f12265a | +0 |
| epic | `649d398b3` | 75b46be9 | +0 |
| epic | `7620257ab` | b22aabce | +0 |
| epic | `70457e502` | fb38bf1f | +37 |
| epic | `7e86df856` | 487f45a9 | -1 |
| epic | `6d9a1ed76` | f35c86de | -3 |
| epic | `6d471aa22` | c06daf50 | +0 |
| epic | `05b00219f` | 487f45a9 | +0 |
| epic | `67f4514c3` | ab168296 | +0 |
| epic | `97d00504e` | 0eca332a | -21 |
| epic | `da2245869` | f9977efd | +0 |
| epic | `e2925fcd8` | 755964b2 | +0 |
| epic | `49c2582ea` | 0546fc82 | +1 |
| epic | `33e7e2d93` | fe3ad1b0 | +0 |
| epic | `3ed24cd50` | 46fe7593 | +0 |
| epic | `ae01461e7` | 0b35be66 | +122 |
| epic | `bed5d7f2a` | afd2d573 | +0 |
| epic | `2318609d5` | 7909527f | +0 |
| epic | `48c408716` | f487aeef | +0 |
| epic | `c24f83489` | f487aeef | +0 |
| epic | `ba3d2b076` | fba8d478 | +0 |
| epic | `38670c86b` | fba8d478 | +0 |
