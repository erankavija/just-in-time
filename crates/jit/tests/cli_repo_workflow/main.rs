//! `cli_repo_workflow` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod apply_cli_tests;
mod archive_preview_cli_tests;
mod artifact_conformance_cli_tests;
mod claim_integration_tests;
mod config_get_tests;
mod cross_substrate_generality_tests;
mod cross_worktree_integration_tests;
mod doc_show_tests;
mod document_history_tests;
mod first_guess_residuals_test;
mod format_compat_cli_tests;
mod gate_preset_tests;
mod help_cross_reference_tests;
mod init_item_kinds_golden;
mod init_tests;
mod integration_schema;
mod integration_test;
mod profile_acceptance_tests;
mod profile_cli_tests;
mod profile_concurrency_tests;
mod profile_edit_refresh_cli_tests;
mod profile_interruption_tests;
mod profile_no_git_lifecycle_tests;
mod project_config_tests;
mod repo_discovery_tests;
mod serve_cli_tests;
mod steering_scenarios;
mod template_binding_cli_tests;
mod test_cli_consistency;
mod version_cli_tests;
mod workflow_tests;
mod worktree_cli_tests;
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
