//! `cli_repo_workflow` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod apply_cli_tests;
mod archive_preview_cli_tests;
mod artifact_conformance_cli_tests;
mod claim_integration_tests;
mod cli_process_fixture_tests;
mod config_get_tests;
mod cross_substrate_generality_tests;
mod cross_worktree_integration_tests;
mod derived_documents_cli_tests;
mod divergence_recovery_journey_tests;
mod doc_show_tests;
mod document_asset_scan_tests;
mod document_history_tests;
mod external_data_root_tests;
mod first_guess_residuals_test;
mod format_compat_cli_tests;
mod gate_preset_tests;
mod git_hook_tests;
mod help_cross_reference_tests;
mod history_isolation_tests;
mod init_item_kinds_golden;
mod init_tests;
mod integration_schema;
mod integration_test;
mod lease_enforcement_tests;
mod linked_checkout_write_policy_tests;
mod lock_contention_tests;
mod profile_acceptance_tests;
mod profile_cli_tests;
mod profile_concurrency_tests;
mod profile_dirty_target_tests;
mod profile_no_git_lifecycle_tests;
mod profile_publication_tests;
mod project_config_tests;
mod repo_discovery_tests;
mod serve_cli_tests;
mod steering_scenarios;
mod template_binding_cli_tests;
mod test_cli_consistency;
mod version_cli_tests;
mod workflow_tests;
mod worktree_cli_tests;
mod worktree_exec_tests;
mod worktree_identity_tests;

/// Stage every profile package this repository authors inside `repo`, and
/// answer with the repository-relative location of `id`.
///
/// A package is applied from inside the worktree it is applied to, and a
/// declared dependency is looked for beside the package declaring it, so the
/// whole authored set is staged and a `path:` selector names one of them.
pub(crate) fn repository_package_at(repo: &std::path::Path, id: &str) -> String {
    let staged = jit::test_utils::stage_repository_packages(repo, id);
    staged
        .strip_prefix(repo)
        .expect("the packages are staged inside the repository")
        .to_string_lossy()
        .into_owned()
}

/// Create an `epic` container, returning its full id.
pub(crate) fn create_epic(cli: &jit::test_cli::JitCli, title: &str) -> String {
    cli.create_issue([
        "--title",
        title,
        "--description",
        "## Success Criteria\n\n- [hard] REQ-01: it works\n",
        "--label",
        "type:epic",
    ])
    .unwrap()
}

/// Install `templates_toml` as `.jit/templates.toml` and define every gate key
/// its templates name; a template's gate entries resolve against the
/// repository's own gate registry.
pub(crate) fn install_templates(cli: &jit::test_cli::JitCli, templates_toml: &str) {
    std::fs::write(cli.root().join(".jit/templates.toml"), templates_toml).unwrap();
    let no_hierarchy_check: [&str; 0] = [];
    jit::templates::TemplateRegistry::from_toml_str(templates_toml, &no_hierarchy_check)
        .expect("the fixture template registry parses")
        .templates
        .iter()
        .flat_map(jit::test_harness::template_gate_keys)
        .collect::<std::collections::BTreeSet<_>>()
        .iter()
        .for_each(|key| {
            cli.run([
                "gate",
                "define",
                key.as_str(),
                "--title",
                &format!("{key} gate"),
                "--description",
                &format!("Repository-declared {key} gate"),
            ])
            .unwrap();
        });
}

pub(crate) const ALLOW_LINKED_WRITES: (&str, &str) = (
    jit::config_manager::LINKED_CHECKOUT_WRITE_STANCE_ENV,
    jit::domain::LinkedCheckoutWriteStance::Allow.as_token(),
);
