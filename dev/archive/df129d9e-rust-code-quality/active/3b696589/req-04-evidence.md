# REQ-04 evidence for 3b696589

The graceful-shutdown suite passed 20 of 20 runs alone and 20 of 20 runs during a full parallel workspace test run. The runs used commit `cef206e33` on 2026-11-04, with `CARGO_INCREMENTAL=0`, under the shared host lock (`flock -o /tmp/jit-ci-turnstile.lock flock -o -s /tmp/jit-full-ci.lock`).

## Commands

- Alone, 20 times in sequence: `cargo test -q -p jit-server --test server_integration graceful_shutdown`.
- Parallel: a background loop ran `cargo test --workspace -q` until the 20 suite runs finished. The suite ran 20 times in sequence through the already-built `server_integration` test binary (`<bin> graceful_shutdown -q`), starting 20 seconds after the workspace run began.

## Raw summaries: alone

```text
commit cef206e33, started 2026-11-04T13:27:45Z
alone run 1: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
alone run 2: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
alone run 3: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
alone run 4: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.41s
alone run 5: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.41s
alone run 6: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.41s
alone run 7: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
alone run 8: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
alone run 9: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.41s
alone run 10: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.41s
alone run 11: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.41s
alone run 12: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.41s
alone run 13: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
alone run 14: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.41s
alone run 15: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
alone run 16: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
alone run 17: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
alone run 18: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
alone run 19: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
alone run 20: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
finished 2026-11-04T13:30:01Z
```

## Raw summaries: parallel

```text
commit cef206e33, started 2026-11-04T13:27:45Z
parallel run 1: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.43s
parallel run 2: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.42s
parallel run 3: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.40s
parallel run 4: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.43s
parallel run 5: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.42s
parallel run 6: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.42s
parallel run 7: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.43s
parallel run 8: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.42s
parallel run 9: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.43s
parallel run 10: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.43s
parallel run 11: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.43s
parallel run 12: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.43s
parallel run 13: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.42s
parallel run 14: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.43s
parallel run 15: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.42s
parallel run 16: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.42s
parallel run 17: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.42s
parallel run 18: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.42s
parallel run 19: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.42s
parallel run 20: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.41s
workspace run 1: exit 0, 17 suites ok, 0 failed
finished 2026-11-04T13:30:01Z
```
