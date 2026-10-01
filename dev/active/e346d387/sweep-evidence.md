# e346d387 sweep evidence

Completion evidence for REQ-04 of jit:e346d387. Every row is a remaining
occurrence of a sweep term in the workspace sources (`crates/`, `mcp-server/`
JavaScript, `web/src/`); no pinned archive lies under these paths. Line numbers
are those of the commit that adds this file.

## Command

```sh
rg -n -i -w "stage|stages|staged|staging|backup|backups|preimage|preimages|journal|journals|journaled|reverse actions?|roll back|rollback|recovered session|one recoverable transaction" \
  crates mcp-server/index.js mcp-server/lib web/src
```

`journal`, `backup`, `reverse action`, `roll back`, `recovered session`, and
`one recoverable transaction` return no occurrence. The retired forms outside
the nine terms (SP-27) return only unrelated senses:

```sh
rg -n -i "recoverable (repository |multi-target )?transaction|recovered (mutation |repository )?(session|store|delta)|recovering kernel|transaction recovery|rollback|roll(ed)? back|companion" \
  crates mcp-server/index.js mcp-server/lib web/src
```

| Location | Form | Sense |
|---|---|---|
| `crates/jit/tests/cli_repo_workflow/divergence_recovery_journey_tests.rs:467` | recovered store | a store restored from git divergence, not publication |
| `crates/jit/tests/cli_query_graph/label_filter_repeatable_query_tests.rs:5` | companion | a companion test file |
| `crates/jit/src/repository_state/default_rules.rs:318` | companion | a companion function |
| `crates/jit/src/commands/gate_check.rs:1445` | companion | a companion function |

## Classes

- **unrelated**: the term's sense is unrelated to publication (gate stages,
  git staging, test fixtures copied into a test repository, validation pipeline
  stages, a captured prior issue value).
- **live-publication**: the term names a publication mechanism production runs
  today. These rows are left for the owner's REQ-04 decision; no identifier or
  sense was reworded to pass the sweep. The live mechanisms are:
  - the apply-time conflict check, typed `ExpectedPreimage` in
    `crates/jit/src/repository_state/image.rs`;
  - the init publisher's sibling stage directory for an absent data root,
    `crates/jit/src/storage/init_publish.rs`;
  - temp-file staging plus rename for a single file write (`@/inv/atomic-writes`),
    in redo application, the PID file, generated-artifact regeneration, and
    test-fixture publication;
  - external export publication, `crates/jit/src/storage/external_publish.rs`.

## Occurrences

| Location | Term | Class | Sense |
|---|---|---|---|
| `web/src/types/models.ts:21` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `web/src/types/models.ts:28` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/fast_rules/project_render_harness_tests.rs:107` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_issue/failure_probe_coverage_tests.rs:234` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_coverage_tests.rs:238` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_coverage_tests.rs:239` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_coverage_tests.rs:240` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_coverage_tests.rs:241` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_coverage_tests.rs:514` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_coverage_tests.rs:561` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_gate/bracket_coverage_gate_run_test.rs:127` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_query_graph/scope_validation_tests.rs:182` | preimage | unrelated | test seed of an exact prior record value into an in-memory image |
| `crates/jit/tests/cli_gate/gate_evaluate_all_test.rs:102` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/fixtures/steering/bracket-plan-rejected/scenario.toml:33` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_gate/gate_update_test.rs:67` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/fast_rules/derived_state_repair_tests.rs:255` | stages | unrelated | test fixture content copied into a test repository / fixture text |
| `web/src/components/Issue/IssueDetail.tsx:511` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/common/harness.rs:235` | preimage | unrelated | test seed of an exact prior record value into an in-memory image |
| `crates/jit/tests/common/harness.rs:241` | preimage | unrelated | test seed of an exact prior record value into an in-memory image |
| `crates/jit/tests/common/harness.rs:318` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/registry_json_tests.rs:136` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/registry_json_tests.rs:146` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/registry_json_tests.rs:155` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/registry_json_tests.rs:158` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/registry_json_tests.rs:170` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/registry_json_tests.rs:179` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/registry_json_tests.rs:208` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/registry_json_tests.rs:216` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/registry_json_tests.rs:225` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/registry_json_tests.rs:226` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/registry_json_tests.rs:236` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/fast_rules/example_rulesets_tests.rs:1603` | preimage | unrelated | test seed of an exact prior record value into an in-memory image |
| `crates/jit/tests/cli_repo_workflow/gate_preset_tests.rs:68` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_issue/exit_code_tests.rs:715` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_issue/exit_code_tests.rs:778` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_gate/gate_cli_integration_test.rs:516` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_gate/gate_cli_integration_test.rs:550` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/item_cli_tests.rs:637` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/test_utils.rs:659` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/test_utils.rs:660` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/test_utils.rs:662` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/test_utils.rs:672` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/test_utils.rs:676` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/test_utils.rs:1068` | staging | unrelated | test fixture package copied into a test worktree |
| `crates/jit/src/test_utils.rs:1076` | staged | unrelated | test fixture package copied into a test worktree |
| `crates/jit/src/test_utils.rs:1078` | staged | unrelated | test fixture package copied into a test worktree |
| `crates/jit/src/test_utils.rs:1082` | staged | unrelated | test fixture package copied into a test worktree |
| `crates/jit/src/test_utils.rs:1114` | stages | unrelated | test fixture package copied into a test worktree |
| `crates/jit/src/test_utils.rs:1584` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/test_utils.rs:1585` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/gate_presets/manager.rs:165` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_item_validate/validate_cli_rule_tests.rs:621` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets/reference.rs:27` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets/reference.rs:40` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets/reference.rs:53` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets/reference.rs:67` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets/reference.rs:81` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_repo_workflow/divergence_recovery_journey_tests.rs:185` | staged | unrelated | git merge stages / git index staging in a divergence-recovery journey test |
| `crates/jit/tests/cli_repo_workflow/divergence_recovery_journey_tests.rs:185` | stage | unrelated | git merge stages / git index staging in a divergence-recovery journey test |
| `crates/jit/tests/cli_repo_workflow/divergence_recovery_journey_tests.rs:186` | stage | unrelated | git merge stages / git index staging in a divergence-recovery journey test |
| `crates/jit/tests/cli_repo_workflow/divergence_recovery_journey_tests.rs:187` | stage | unrelated | git merge stages / git index staging in a divergence-recovery journey test |
| `crates/jit/tests/cli_repo_workflow/divergence_recovery_journey_tests.rs:190` | stage | unrelated | git merge stages / git index staging in a divergence-recovery journey test |
| `crates/jit/tests/cli_repo_workflow/divergence_recovery_journey_tests.rs:190` | stage | unrelated | git merge stages / git index staging in a divergence-recovery journey test |
| `crates/jit/tests/cli_repo_workflow/divergence_recovery_journey_tests.rs:207` | stages | unrelated | git merge stages / git index staging in a divergence-recovery journey test |
| `crates/jit/tests/cli_repo_workflow/divergence_recovery_journey_tests.rs:244` | stage | unrelated | git merge stages / git index staging in a divergence-recovery journey test |
| `crates/jit/tests/cli_repo_workflow/divergence_recovery_journey_tests.rs:244` | stage | unrelated | git merge stages / git index staging in a divergence-recovery journey test |
| `crates/jit/tests/cli_repo_workflow/divergence_recovery_journey_tests.rs:250` | staging | unrelated | git merge stages / git index staging in a divergence-recovery journey test |
| `crates/jit/tests/cli_repo_workflow/main.rs:43` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_repo_workflow/main.rs:48` | staged | unrelated | test fixture content copied into a test repository / fixture text |
| `crates/jit/tests/cli_repo_workflow/main.rs:50` | staged | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_repo_workflow/main.rs:51` | staged | unrelated | test fixture content copied into a test repository / fixture text |
| `crates/jit/tests/cli_repo_workflow/main.rs:53` | staged | unrelated | test fixture content copied into a test repository / fixture text |
| `crates/jit/tests/cli_issue/command_exit_code_projection_tests.rs:912` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/cli_issue/failure_probe_fixture.rs:341` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_fixture.rs:348` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_fixture.rs:365` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_fixture.rs:368` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_fixture.rs:368` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_fixture.rs:369` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_fixture.rs:370` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_fixture.rs:447` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_fixture.rs:448` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_fixture.rs:449` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_fixture.rs:450` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_issue/failure_probe_fixture.rs:1053` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/tests/cli_repo_workflow/repo_discovery_tests.rs:165` | staged | unrelated | test fixture package copied into a test repository |
| `crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:236` | staged | unrelated | test fixture content copied into a test repository / fixture text |
| `crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:346` | stages | unrelated | test fixture content copied into a test repository / fixture text |
| `crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:1663` | stage | unrelated | test fixture content copied into a test repository / fixture text |
| `crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:1668` | staged | unrelated | test fixture content copied into a test repository / fixture text |
| `crates/jit/tests/scratch_build/repository_state_cutover_tests.rs:710` | staged | live-publication | cutover guard text: init publisher stages an absent data root beside it |
| `crates/jit/tests/scratch_build/repository_state_cutover_tests.rs:765` | staged | unrelated | cutover guard text: snapshot export assembles in a private temp directory |
| `crates/jit/src/storage/gate_store.rs:53` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/gate_store.rs:63` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/init_publish.rs:3` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:9` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:21` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:34` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:37` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:41` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:46` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:55` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:56` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:57` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:58` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:89` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:93` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:104` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:105` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:108` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:117` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:122` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:123` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:124` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:125` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/init_publish.rs:132` | staged | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/redo_apply.rs:236` | staged | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:237` | staged | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:238` | staged | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:242` | stage | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:243` | staging | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:250` | staging | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:808` | stage | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:818` | staged | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:879` | staging | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:884` | staged | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:892` | staging | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:897` | staging | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:900` | staging | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:915` | staging | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:921` | staging | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/redo_apply.rs:924` | staging | live-publication | redo application writes each put to a staged temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/storage/mutation_interruption.rs:19` | staged | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/mutation_interruption.rs:20` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/tests/fast_docs_templates/template_apply_atomicity_tests.rs:6` | preimages | live-publication | conflict check: captured preimages each action is revalidated against at apply |
| `crates/jit/tests/fast_issue/transition_graph_enforcement_tests.rs:35` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/repository_state/initialize.rs:422` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/initialize.rs:430` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/initialize.rs:524` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/initialize.rs:616` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/initialize.rs:735` | preimages | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/initialize.rs:788` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/initialize.rs:1305` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/gate_presets.rs:37` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets.rs:38` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets.rs:54` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets.rs:54` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets.rs:210` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets.rs:226` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets.rs:240` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets.rs:299` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets.rs:323` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets.rs:331` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_presets.rs:356` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_execution.rs:34` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_execution.rs:38` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_execution.rs:92` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_execution.rs:107` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_execution.rs:162` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_execution.rs:648` | staged | unrelated | git index staging in a dirty-worktree probe |
| `crates/jit/src/gate_execution.rs:836` | staged | unrelated | git index staging in a dirty-worktree probe |
| `crates/jit/src/gate_execution.rs:1237` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_execution.rs:1259` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/gate_execution.rs:1500` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/domain/types.rs:944` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/domain/types.rs:945` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/tests/fast_docs_templates/template_apply_tests.rs:1081` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/external_publish.rs:106` | staging | live-publication | external export publication: staged file/tree published no-replace to a target outside the repository |
| `crates/jit/src/storage/external_publish.rs:113` | staged | live-publication | external export publication: staged file/tree published no-replace to a target outside the repository |
| `crates/jit/src/storage/external_publish.rs:177` | staged | live-publication | external export publication: staged file/tree published no-replace to a target outside the repository |
| `crates/jit/src/storage/external_publish.rs:188` | stage | live-publication | external export publication: staged file/tree published no-replace to a target outside the repository |
| `crates/jit/src/storage/external_publish.rs:191` | stage | live-publication | external export publication: staged file/tree published no-replace to a target outside the repository |
| `crates/jit/src/storage/external_publish.rs:194` | stage | live-publication | external export publication: staged file/tree published no-replace to a target outside the repository |
| `crates/jit/src/storage/external_publish.rs:205` | stage | live-publication | external export publication: staged file/tree published no-replace to a target outside the repository |
| `crates/jit/src/storage/external_publish.rs:209` | stage | live-publication | external export publication: staged file/tree published no-replace to a target outside the repository |
| `crates/jit/src/storage/external_publish.rs:293` | stage | live-publication | external export publication: staged file/tree published no-replace to a target outside the repository |
| `crates/jit/src/commands/issue.rs:881` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/issue.rs:980` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/issue.rs:1039` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/issue.rs:1276` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/snapshot.rs:232` | stage | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:560` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:568` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:637` | staged | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:643` | staged | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:662` | staged | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:663` | staged | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:664` | staged | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:690` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:691` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:703` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:704` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:715` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:725` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1017` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1018` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1019` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1020` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1021` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1023` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1037` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1038` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1047` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1048` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1053` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1054` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1066` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1070` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/src/commands/snapshot.rs:1071` | staging | unrelated | snapshot export assembles its tree in a private temp directory before archiving |
| `crates/jit/examples/regenerate.rs:101` | staged | live-publication | generated-artifact regeneration writes a temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/examples/regenerate.rs:102` | staged | live-publication | generated-artifact regeneration writes a temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/examples/regenerate.rs:106` | staged | live-publication | generated-artifact regeneration writes a temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/examples/regenerate.rs:110` | stage | live-publication | generated-artifact regeneration writes a temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/examples/regenerate.rs:111` | staged | live-publication | generated-artifact regeneration writes a temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/examples/regenerate.rs:113` | staged | live-publication | generated-artifact regeneration writes a temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/examples/regenerate.rs:117` | staged | live-publication | generated-artifact regeneration writes a temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/examples/regenerate.rs:120` | staged | live-publication | generated-artifact regeneration writes a temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/examples/regenerate.rs:121` | staged | live-publication | generated-artifact regeneration writes a temp file and renames it into place (@/inv/atomic-writes) |
| `crates/jit/src/output.rs:2448` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:2479` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:2550` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:2550` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:2722` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:2732` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:2733` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:2754` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:2754` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:2971` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:3180` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:3950` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:3956` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:3956` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:3965` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:3971` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:3983` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:3993` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/output.rs:3996` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/repository_state/path.rs:15` | stage | live-publication | init publisher's sibling stage directory |
| `crates/jit/src/repository_state/path.rs:20` | staging | live-publication | file writes publish by temp-file staging plus rename (@/inv/atomic-writes) |
| `crates/jit/src/commands/serve.rs:1463` | stages | live-publication | server PID file atomic write stages its temp file before rename (@/inv/atomic-writes) |
| `crates/jit/src/commands/serve.rs:1465` | staging | live-publication | server PID file atomic write stages its temp file before rename (@/inv/atomic-writes) |
| `crates/jit/src/commands/serve.rs:1499` | staged | live-publication | server PID file atomic write stages its temp file before rename (@/inv/atomic-writes) |
| `crates/jit/src/commands/gate_cli_tests.rs:28` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_cli_tests.rs:50` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_cli_tests.rs:55` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_cli_tests.rs:209` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/test_support.rs:18` | staged | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/storage/test_support.rs:28` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/storage/test_support.rs:33` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/storage/test_support.rs:37` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/storage/test_support.rs:46` | staged | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/storage/test_support.rs:51` | staging | live-publication | test-fixture publication: verified staging plus atomic no-replace publication (@/inv/atomic-writes) |
| `crates/jit/src/storage/test_support.rs:182` | preimage | unrelated | test seed of an exact prior record value into an in-memory image |
| `crates/jit/src/storage/reference.rs:72` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/reference.rs:73` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/reference.rs:118` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/reference.rs:145` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/reference.rs:145` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/reference.rs:184` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/reference.rs:184` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/reference.rs:328` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/gate_runs.rs:69` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/gate_runs.rs:139` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/gate_runs.rs:167` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/gate_runs.rs:200` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/gate_runs.rs:237` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/gate_runs.rs:299` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/lock_cleanup.rs:48` | stage | unrelated | two-stage stale-lock detection |
| `crates/jit/src/storage/repository_state_store.rs:360` | staged | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store.rs:388` | stages | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store.rs:634` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store.rs:639` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store.rs:666` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:819` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/storage/repository_state_store_tests.rs:839` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/storage/repository_state_store_tests.rs:846` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/storage/repository_state_store_tests.rs:852` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/storage/repository_state_store_tests.rs:1436` | staged | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:1437` | staging | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:1437` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:2327` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/repository_state_store_tests.rs:2534` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/storage/repository_state_store_tests.rs:3783` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3789` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3793` | staged | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3793` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3798` | staged | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3813` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3816` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3816` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3847` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3852` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3871` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3872` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3884` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3885` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/repository_state_store_tests.rs:3889` | stage | live-publication | init publisher: an absent data root is built in a sibling stage directory and published by no-replace rename |
| `crates/jit/src/storage/redo_record.rs:197` | staged | live-publication | states the redo record is written in place rather than staged and renamed |
| `crates/jit/src/domain/item.rs:1218` | stage | unrelated | doc example text: issue lifecycle stage |
| `crates/jit/src/storage/memory.rs:317` | stage | unrelated | test helper places a config-declared file in the in-memory image |
| `crates/jit/src/repository_state/image.rs:1337` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:1339` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:1343` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:1390` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:1423` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:1575` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:1614` | preimages | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:1624` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:1691` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:1694` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:1697` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:1700` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:2303` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:2322` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:2322` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:2352` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image.rs:2377` | stage | unrelated | test path component below the bootstrap lock |
| `crates/jit/src/repository_state/image.rs:2530` | preimages | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image_plan_hash_property_tests.rs:57` | preimages | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image_plan_hash_property_tests.rs:111` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image_plan_hash_property_tests.rs:112` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/image_plan_hash_property_tests.rs:282` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/mod.rs:113` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/mod.rs:941` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/mod.rs:942` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/mod.rs:965` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/mod.rs:968` | preimages | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/repository_state/mod.rs:1547` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/cli.rs:42` | staged | live-publication | `jit init` help: an absent data root is staged complete and published no-replace (init publisher) |
| `crates/jit/src/cli.rs:1353` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/cli.rs:1355` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/cli.rs:1429` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/cli.rs:1431` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/repository_state/rules_gates_projection.rs:240` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/validation/repository.rs:29` | stage | unrelated | named pipeline stage of whole-repository validation |
| `crates/jit/src/validation/repository.rs:50` | stages | unrelated | named pipeline stage of whole-repository validation |
| `crates/jit/src/validation/repository.rs:53` | stages | unrelated | named pipeline stage of whole-repository validation |
| `crates/jit/src/repository_state/mutation.rs:395` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:402` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:940` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:945` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:949` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:950` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:951` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:952` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:953` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:956` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:959` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:981` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:992` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps |
| `crates/jit/src/repository_state/mutation.rs:995` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:997` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:1168` | preimage | live-publication | apply-time conflict check: ExpectedPreimage derived from the captured image |
| `crates/jit/src/repository_state/mutation.rs:1334` | preimage | live-publication | apply-time conflict check: ExpectedPreimage derived from the captured image |
| `crates/jit/src/repository_state/mutation.rs:1350` | preimage | live-publication | apply-time conflict check: ExpectedPreimage derived from the captured image |
| `crates/jit/src/repository_state/mutation.rs:1358` | preimage | live-publication | apply-time conflict check: ExpectedPreimage derived from the captured image |
| `crates/jit/src/repository_state/mutation.rs:2007` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:2008` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:2016` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:2026` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:2445` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:2446` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:2447` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:2448` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:2449` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:2451` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:2458` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:2481` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/repository_state/mutation.rs:2487` | preimage | unrelated | captured prior value of an issue, read by the finalizer to preserve lifecycle timestamps (or a test's seeded prior issue) |
| `crates/jit/src/commands/invariant.rs:94` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/repository_state/profile_apply.rs:1063` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/commands/config.rs:310` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/declarations/gates.rs:9` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/declarations/gates.rs:12` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/declarations/gates.rs:13` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/declarations/gates.rs:17` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/declarations/gates.rs:164` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/declarations/gates.rs:165` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/declarations/gates.rs:203` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/declarations/gates.rs:214` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/declarations/gates.rs:214` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/declarations/gates.rs:296` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/declarations/gates.rs:303` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/declarations/gates.rs:320` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/declarations/gates.rs:385` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:181` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:182` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:209` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:430` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:431` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:629` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:629` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:629` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:1161` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:1175` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:1192` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:1206` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:1238` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:1519` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:1519` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:1574` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:1633` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:1707` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:1769` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:2027` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:2237` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate.rs:2294` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/item.rs:1363` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/item.rs:1369` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/item.rs:1448` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/mod.rs:626` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/mod.rs:1498` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/commands/mod.rs:3497` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/mod.rs:3522` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/mod.rs:3579` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/mod.rs:3652` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/mod.rs:3821` | preimage | unrelated | captured prior value of an issue whose event identity and timestamp are retained |
| `crates/jit/src/commands/init.rs:67` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/commands/init.rs:69` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/commands/init.rs:77` | staged | live-publication | init publisher's staged-root publication of an absent data directory |
| `crates/jit/src/commands/init.rs:429` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/commands/init.rs:450` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/commands/init.rs:818` | stages | unrelated | test fixture package copied into a test repository |
| `crates/jit/src/commands/init.rs:1074` | staged | unrelated | test fixture package copied into a test repository |
| `crates/jit/src/commands/init.rs:1515` | staged | unrelated | test fixture package copied into a test repository |
| `crates/jit/src/commands/gate_check.rs:36` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:43` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:953` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:1076` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:1226` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:1335` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:1335` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:1365` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:1376` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:1502` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:1618` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:1692` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:1777` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:1800` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:1923` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:2236` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3164` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3217` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3257` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3282` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3358` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3436` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3461` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3522` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3579` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3619` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3665` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3722` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3770` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3825` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3886` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:3955` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:4017` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:4094` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:4641` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:4718` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:4766` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:4823` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:4871` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:4919` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:4984` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:5055` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:5105` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:5167` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:5217` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/gate_check.rs:5233` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/profile.rs:1003` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/commands/profile.rs:1111` | staging | unrelated | profile pack writes the archive to a temp file before handing it to the publisher |
| `crates/jit/src/commands/profile.rs:1116` | staged | unrelated | profile pack writes the archive to a temp file before handing it to the publisher |
| `crates/jit/src/commands/profile.rs:1117` | staged | unrelated | profile pack writes the archive to a temp file before handing it to the publisher |
| `crates/jit/src/commands/profile.rs:1118` | staged | unrelated | profile pack writes the archive to a temp file before handing it to the publisher |
| `crates/jit/src/commands/profile.rs:1122` | staged | unrelated | profile pack writes the archive to a temp file before handing it to the publisher |
| `crates/jit/src/commands/profile.rs:3116` | stages | unrelated | test fixture package copied into a test repository / test path name |
| `crates/jit/src/commands/profile.rs:3121` | stages | unrelated | test fixture package copied into a test repository / test path name |
| `crates/jit/src/commands/profile.rs:3128` | staged | unrelated | test fixture package copied into a test repository / test path name |
| `crates/jit/src/commands/profile.rs:3358` | staging | unrelated | test fixture package copied into a test repository / test path name |
| `crates/jit/src/commands/profile.rs:3386` | staged | unrelated | test fixture package copied into a test repository / test path name |
| `crates/jit/src/commands/profile.rs:8313` | staging | unrelated | test fixture package copied into a test repository / test path name |
| `crates/jit/src/commands/profile.rs:8321` | staging | unrelated | test fixture package copied into a test repository / test path name |
| `crates/jit/src/commands/profile.rs:8324` | staging | unrelated | test fixture package copied into a test repository / test path name |
| `crates/jit/src/commands/validate.rs:179` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/commands/validate.rs:1215` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/commands/validate.rs:3206` | staged | unrelated | test fixture package copied into a test repository |
| `crates/jit/src/commands/validate.rs:3748` | staged | unrelated | test fixture package copied into a test repository |
| `crates/jit/src/commands/validate.rs:3752` | staged | unrelated | test fixture package copied into a test repository |
| `crates/jit/src/commands/validate.rs:3755` | staged | unrelated | test fixture package copied into a test repository |
| `crates/jit/src/commands/validate.rs:3756` | staged | unrelated | test fixture package copied into a test repository |
| `crates/jit/src/commands/validate.rs:3758` | staged | unrelated | test fixture package copied into a test repository |
| `crates/jit/src/commands/validate.rs:4419` | stages | unrelated | test fixture package copied into a test repository |
| `crates/jit/src/main.rs:314` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/main.rs:1564` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/main.rs:2589` | preimage | live-publication | apply-time conflict check: each action's ExpectedPreimage (captured occupant) is revalidated against the live path |
| `crates/jit/src/main.rs:4223` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/main.rs:4302` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/main.rs:4336` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/main.rs:4421` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/main.rs:4497` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/main.rs:4524` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/main.rs:4524` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/main.rs:5408` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
| `crates/jit/src/main.rs:8977` | stage | unrelated | gate execution stage (precheck/postcheck), a gate definition field |
