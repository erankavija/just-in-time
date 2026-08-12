# Applied-profile library baseline evidence (`ae17798d`)

This directory closes REQ-04's measurement gap without changing the candidate,
its nextest filters, or either setup recipe. The one warmup and three measured
runs use the exact ten `jit::jit` identities selected by the existing default
and dogfood setup filters. All Cargo work was serialized by an exclusive
`/tmp/cargo-ci.lock` and used the authorized shared main target because `/tmp`
had only 3.4 GiB free while the existing target occupied 14 GiB.

The stable pre-change reference is not a recollection: the validator extracts
the same ten identities from committed `dev/benchmarks/suite-profile.json`
(artifact commit `58928c35`, profiled revision `6c536622`) and rederives their
64,894 ms sum. It then parses every raw candidate event and setup observation,
checks exact identity equality, rejects duplicates or non-passing results, and
recomputes all spans and reductions.

The established source profile used one warmup, so the colder candidate warmup
is retained but excluded from the decision. Each of the three measured runs
passes both REQ-04 thresholds:

| Sample | Default setup | Dogfood setup | Test sum | Setup-inclusive span | Reduction | Command wall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 2,608 ms | 2,764 ms | 16,623 ms | 21,995 ms | 42,899 ms | 9,602 ms |
| 2 | 2,643 ms | 2,711 ms | 16,678 ms | 22,032 ms | 42,862 ms | 9,610 ms |
| 3 | 2,593 ms | 2,746 ms | 16,693 ms | 22,032 ms | 42,862 ms | 9,631 ms |

Run `python3 dev/benchmarks/applied-profile-baselines-ae17798d/validate.py`
from the repository root to rederive the evidence. `benchmark.py` is the
fail-closed one-shot capture driver and refuses to overwrite the raw records.
