# Production-line baseline for crates/jit/src (df129d9e)

The before figure for the epic's REQ-10 delta. The counter is [`scripts/production-lines.py`](../../../scripts/production-lines.py), committed unchanged from the planning copy [prodlines.py](prodlines.py).

## Measured commit

`dc2ea8706edd285b09746ec7f111ce405845e961`, on `main` before any df129d9e commit changes `crates/jit/src`. Concurrent epics move the figure, so the completion report counts the after figure with the same commands at its own named commit.

## Commands

Run from the repository root:

```sh
python3 scripts/production-lines.py dc2ea8706edd285b09746ec7f111ce405845e961
python3 scripts/production-lines.py dc2ea8706edd285b09746ec7f111ce405845e961 --strip-line-comments --test-support-as-test
```

The counter reads the tree at the named revision through `git show`, so the working tree does not affect the result.

## Figures

**Default** is the authoritative mode: the method the epic's Background describes, excluding blank lines, doc comments, `#[cfg(test)]` items, test-only sibling modules and test-named files. **Flagged** adds `--strip-line-comments --test-support-as-test`, which also drops plain `//` comments and `test-support`-gated code; it is the mode that reproduced the survey's figure of about 63,100 at 21f9e563. A dash marks a module the mode excludes entirely.

| Module | Default (authoritative) | Flagged |
|---|---:|---:|
| `commands` | 21113 | 19926 |
| `repository_state` | 10620 | 10325 |
| `storage` | 9727 | 9313 |
| `main` | 7965 | 7366 |
| `domain` | 7483 | 7347 |
| `profile` | 5041 | 4370 |
| `validation` | 2952 | 2763 |
| `output` | 1839 | 1782 |
| `cli` | 1672 | 1661 |
| `declarations` | 1369 | 1303 |
| `document` | 1237 | 1174 |
| `config` | 1149 | 1105 |
| `test_utils` | 1029 | — |
| `schema` | 1013 | 912 |
| `graph` | 831 | 781 |
| `errors` | 734 | 728 |
| `gate_execution` | 457 | 406 |
| `gate_presets` | 444 | 421 |
| `query_engine` | 375 | 366 |
| `templates` | 366 | 353 |
| `visualization` | 177 | 172 |
| `test_taxonomy` | 151 | — |
| `config_manager` | 149 | 141 |
| `generated_artifacts` | 148 | — |
| `snapshot` | 142 | 141 |
| `search` | 140 | 129 |
| `labels` | 112 | 100 |
| `agent_config` | 88 | 79 |
| `output_macros` | 71 | 68 |
| `runtime_defaults` | 62 | 53 |
| `lib` | 49 | 42 |
| `type_icons` | 46 | 42 |
| `build_info` | 23 | 23 |
| `test_mode` | 15 | 3 |
| `tar_format` | 12 | 12 |
| **Total** | **78801** | **73407** |
