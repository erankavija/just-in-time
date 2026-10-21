# Reproduction verification, epic 8376299c REQ-02

Binary: release `jit` / `jit-server` built from main 0805ad781754 (`which jit` = scratchpad/release/jit).
All repros ran in fresh `mktemp -d` repositories under scratchpad/repro/ (`git init`, `jit init`). Nothing was run mutating in the real repository.
Summary: 13 CORRECTED, 0 NOT CORRECTED, 1 N/A (6ff14f21, policy test shown). See notes on 34999aad (server search coverage) and cd2ed136 (scope shape).

## 2d10f1dd update/reject atomicity
```sh
jit gate define review -t Review -d r
jit issue update $ID --add-gate review --assignee agent:w1 --label BADLABEL   # exit 4
jit issue show $ID; wc -l .jit/events.jsonl
jit issue update $ID --add-gate review --assignee agent:w1 --label component:ok
jit issue reject $RID --reason dup
```
- Rejected update: `Blocked by validation rule(s) ... [label-format]`, exit=4; Assignee: None, Gates: (none); events.jsonl still 2 lines (no gate_added / issue_claimed).
- Valid combined update exit 0: Assignee agent:w1, gate review [Pending], events gate_added, assignee change, fields[labels] share one timestamp.
- Reject: state Rejected, labels `["resolution:dup"]`.
- Not exercised: reject failing at the label step (no CLI way to force it).
Verdict: CORRECTED. The documented failing update leaves issue and log untouched.

## 05d1b34e bulk terminal update cascade
```sh
jit dep add A B; jit issue update B --state ready
jit issue update --filter 'state:ready' --state done      # A: backlog -> ready
# rejected variant: C depends on D; --state rejected      # C -> Ready
# no terminal: E depends on F; --filter state:ready --priority high   # E stays Backlog
jit query available --json    # A present
```
Verdict: CORRECTED. Dependents reach Ready for done and rejected; non-terminal bulk update leaves them unchanged.

## 0fa1e09e server relative data dir
```sh
jit-server --bind 127.0.0.1:38917 &       # default .jit, cwd = repo
curl 127.0.0.1:38917/api/issues           # 200, issue list
cd sub && jit-server --bind ... --data-dir ../.jit &   # vs absolute --data-dir
```
- Default `.jit`: 200 with issues. `../.jit`: 200, 2 issues; absolute path: 200, 2 issues. Servers killed; no listeners left.
Verdict: CORRECTED.

## cd2ed136 graph export scope and order
Repo with `[type_hierarchy] types={epic=2,task=4}`; Task2 depends only on out-of-scope OtherEpic; Epic2 depends on Task2.
```sh
jit graph export --scope $E2 --json | --format dot | --format mermaid
for f in dot mermaid json; do for i in 1..5; do jit graph export --format $f | md5sum; done | sort | uniq -c; done
```
- Scoped (all three formats): nodes = Task2, Epic2 (member whose only dependency is outside scope is listed; OtherEpic excluded). Scope uses containment closure, so the issue's literal "Outside" task sits inside E's closure when both are tasks; the out-of-scope-dependency case needs a more-strategic dependency, as above.
- Unscoped: 8 nodes = 8 issues. Five runs per format give one distinct md5 each (`5 <hash>` x3).
Verdict: CORRECTED.

## 7d1bfe89 non-ASCII filter lexer
```sh
jit issue update --filter 'label:ä' --priority low                       # No issues matched filter, exit 0
jit issue update --filter 'label:ä state:ready' --priority low           # exit 0
jit issue update --filter 'label:ä AND (state:ready OR priority:high)' --priority low   # exit 0
jit issue update --filter 'état:é ü' ...   # Error: Unknown filter field: 'état' ..., exit 1
jit issue update --filter 'label:日本語 %' ...  # Error: Invalid filter '%' ..., exit 1
```
Verdict: CORRECTED. No panic (no exit 101); invalid filters give an error status and message.

## 34999aad ripgrep argv
```sh
jit search 'fla.' --regex --context 1 --json     # count 2
jit search 'fla.' --regex -g '*.json' --json     # count 1, issue file line
jit search --json -- '--verbose'                 # count 2
jit search --json -- '-flag'                     # count 2
jit search --regex --json -- '--verb.se'         # count 2
# HTTP: curl 'host/api/search?q=find.e&regex=true' (doc attached via jit doc add)
```
- CLI: all four reproductions return matching lines, no SEARCH_FAILED.
- HTTP with regex (also with case_sensitive): 200 with the matching document line; `q=-nomatch&regex=true`: 200, total 0.
- Note: the HTTP route returns 0 results for issue JSON content (even literal `q=Hello`, regex off). rg skips the hidden `.jit` dir under the worktree root for the `.jit/issues/*.json` glob (`rg --glob '.jit/issues/*.json'` finds nothing without `--hidden`). This is unrelated to the regex/dash fix and outside the issue's criteria, but means server search never matches issue files in the default `.jit` layout.
Verdict: CORRECTED (criteria met; note above).

## aa423d02 claim-next --filter
```sh
# Alpha task critical, Beta epic low, both ready
jit issue claim-next agent:w1 --filter 'label:type:epic' --json   # claims Beta (in_progress, agent:w1)
jit issue claim-next agent:w2 --filter 'garbage %%%' --json       # exit 2, INVALID_ARGUMENT
jit issue claim-next agent:w3 --filter 'label:type:story' --json  # exit 3, ISSUE_NOT_FOUND
```
- Alpha stayed ready/unassigned. `--help` documents `--filter` with syntax; core-model.md describes filtered claiming (lines ~1164-1178, 1246-1254).
Verdict: CORRECTED.

## c0d0c8bf search label wildcard
```sh
jit query all --label 'type:*' --json          # count 2
jit issue search --label 'type:*' --json       # count 2 (T1, E1)
jit issue search --label 'type:task' --json    # count 1
jit issue search T --label 'type:*' --json     # count 1
jit issue search --label 'component:*' --json  # count 1 (matches query all)
jit issue search --label type | ':*' | 'type:' # exit 2 INVALID_LABEL_PATTERN
```
- Help: "exact `namespace:value` or `namespace:*` wildcard".
Verdict: CORRECTED.

## 514df7d3 pre-commit hook paths
Setup: `jit hooks install` in scratch repo, `[worktree] enforce_leases = "strict"` committed on main, `feature` and `main` each one commit ahead of the merge base.
```sh
echo '# x' >> .jit/gates.toml;  git add; git commit -m g   # blocked, exit 1
echo '# x' >> .jit/config.toml; git add; git commit -m c   # blocked
jit issue create x; git add .jit/issues; JIT_AGENT_ID=agent:nobody git commit -m i
```
- gates.toml and config.toml: "Pre-commit hook: Global .jit/ changes require common history with main".
- Issue file: "Structural edit to issue <uuid> without active lease", exit 1.
- Installed hook header cites `.jit/config.toml, .jit/gates.toml` and `.jit/issues/<uuid>.json`; `git grep` for `gates/registry.json`, `type-hierarchy`, `issues/<id>/issue.json` in scripts/ at 0805ad781754 finds none.
Verdict: CORRECTED.

## 6ff14f21 JIT_TEST_MODE races
No deterministic reproduction (scheduling race). Policy test present:
`crates/jit/tests/scratch_build/process_state_policy_tests.rs:61: fn test_workspace_mutates_process_state_only_in_the_shared_guards()`
Verdict: N/A (race, no deterministic reproduction); policy test exists in the tree.

## d23c58c0 orphan-leaf membership namespaces
Config: `types={release=1,task=4}`, `label_associations release="milestone"`; custom rule `orphan-leaf` (`assert = { type-hierarchy = { kind = "orphan-leaf" } }`, severity warn) because the shipped default has no such rule.
```sh
jit issue create Good --type task --label milestone:v1
jit issue create Bad  --type task --label release:v1
jit issue create Orph --type task
jit validate
```
- Warnings only for Bad (release:v1) and Orph; none for Good (milestone:v1). Warnings: 2.
Verdict: CORRECTED.

## d7d0d0db expired-lease re-eviction
```sh
jit claim acquire $I --ttl 1 --agent-id agent:w1; sleep 3
jit claim list --json   # x3 -> count 0 each
grep -c auto-evict .git/jit/claims.jsonl   # 1
python3 -c '... claims.index.json ["leases"]'   # []
# then: claim list + claim status with no expired leases -> claims.jsonl 2 lines before and after
```
Verdict: CORRECTED. One eviction record, index empty, no growth on later reads.

## d92f54aa inert coordination keys (rescoped)
```sh
printf '\n[coordination]\n%s = 60\n' $k >> .jit/config.toml   # k in stale_threshold_secs, default_ttl_secs, lease_renewal_threshold_pct
jit config show --json; jit issue create x; jit validate
```
- Each key: `Failed to parse config.toml` / `config.toml is invalid: TOML parse error ... unknown field '<key>', expected max_indefinite_leases_per_agent or max_indefinite_leases_per_repo`.
- Valid repo: `jit config show --json` lists no stale_threshold/default_ttl/renewal key; `claims.index.json` after `--ttl 0 --reason probe` has no per-index threshold, lease `stale:false`.
Verdict: CORRECTED.

## bedf0962 enforce_leases reporting vs enforcement
Probe: `jit config show --json` enforce_leases vs `jit issue update $id --priority high` without a lease.
| state | reported | update |
|---|---|---|
| no [worktree] | off | succeeds |
| section, no key | strict | No active lease error |
| explicit off / warn / strict | off / warn / strict | succeeds / warning / error |
| no section + JIT_ENFORCE_LEASES=strict | off | succeeds |
| section, no key + env=off | strict | error |
| explicit strict + env=off | strict | error |
| user layer (`~/.config/jit/config.toml` strict via fake HOME), no section | off | succeeds |
- Reported and enforced mode agree in all states; env var and user layer change neither. configuration.md states the single repo-only rule (lines ~491-495, priority line 29).
Verdict: CORRECTED.

## Addendum: 16d63038 and b5faef84 (release build from main c0517b5af)

### 16d63038 id resolution
```sh
jit issue show|update|gate evaluate <UPPERCASE full id>      # exit 0 on all three
jit issue show|update|gate evaluate <hyphen-free full id>    # exit 0 on all three
jit issue show|update|gate evaluate <31-char prefix>         # exit 0 on all three
jit issue show abc --json                                    # INVALID_ID_PREFIX
```
Verdict: CORRECTED. Full ids resolve in any case and hyphenation on every command, and a below-minimum prefix fails with the invalid-prefix error.

### b5faef84 server search over nested `.jit`
```sh
jit init && jit issue create --title Hello
jit-server --bind 127.0.0.1:38931 &
curl -s '127.0.0.1:38931/api/search?q=Hello'   # total 1, match in .jit/issues/<id>.json
```
Verdict: CORRECTED. The search endpoint returns the issue-record match with the default nested data directory.

Overall: 15 CORRECTED, 0 NOT CORRECTED, 1 N/A (6ff14f21).
