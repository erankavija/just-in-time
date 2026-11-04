//! `jit hooks` handlers.

use super::{CommandContext, Outcome};
use crate::{cli::HooksCommands, output::hooks::HooksInstallResponse, storage::IssueStore};

const NOT_GIT_REPOSITORY: &str = "Not in a git repository (no .git directory found)";

fn install<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
) -> anyhow::Result<Outcome<HooksInstallResponse>> {
    let paths = ctx.executor.require_worktree_paths()?;
    anyhow::ensure!(
        paths.is_git_repository(),
        crate::errors::InvalidArgumentError::new(NOT_GIT_REPOSITORY)
    );

    let result = crate::commands::hooks::install_hooks(&paths.common_dir)?;
    let (hooks_dir, installed, skipped) = (result.hooks_dir, result.installed, result.skipped);
    let message = format!("Installed {} hook(s) to {}", installed.len(), hooks_dir);
    Ok(Outcome::success_with(
        HooksInstallResponse {
            hooks_dir,
            installed,
            skipped,
        },
        message,
    ))
}

command_group! { command: HooksCommands, storage: [IssueStore], leaves: {
    "hooks_install" => HooksCommands::Install => install() -> HooksInstallResponse,
} }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::output::failure::classify;
    use crate::output::{ErrorCode, ExitCode, OutputMode};
    use crate::storage::{InMemoryStorage, WorktreePaths};
    use tempfile::TempDir;

    fn paths(root: &std::path::Path, git_repository: bool) -> WorktreePaths {
        WorktreePaths {
            git_repository,
            common_dir: root.to_path_buf(),
            worktree_root: root.to_path_buf(),
            local_jit: root.join(".jit"),
            shared_jit: root.join("jit"),
            relocated_worktree_checkout: None,
        }
    }

    fn executor(root: &std::path::Path, git_repository: bool) -> CommandExecutor<InMemoryStorage> {
        CommandExecutor::new(InMemoryStorage::new())
            .with_worktree_paths(paths(root, git_repository))
    }

    #[test]
    fn test_dispatch_install_json_preserves_success_payload_and_message() {
        let temp = TempDir::new().unwrap();
        let executor = executor(temp.path(), true);
        let ctx = CommandContext {
            executor: &executor,
            mode: OutputMode {
                json: true,
                quiet: false,
            },
        };
        let mut out = Vec::new();
        let mut err = Vec::new();

        let status = dispatch(HooksCommands::Install, &ctx, &mut out, &mut err).unwrap();

        let printed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let hooks_dir = temp.path().join("hooks").display().to_string();
        assert_eq!(status, ExitCode::Success);
        assert_eq!(printed["hooks_dir"], hooks_dir);
        assert_eq!(printed["installed"], serde_json::json!(["pre-commit"]));
        assert_eq!(printed["skipped"], serde_json::json!([]));
        assert_eq!(
            printed["message"],
            format!("Installed 1 hook(s) to {hooks_dir}")
        );
        assert!(err.is_empty());
    }

    #[test]
    fn test_dispatch_install_human_preserves_success_rendering() {
        let temp = TempDir::new().unwrap();
        let executor = executor(temp.path(), true);
        let ctx = CommandContext {
            executor: &executor,
            mode: OutputMode {
                json: false,
                quiet: false,
            },
        };
        let mut out = Vec::new();

        let status = dispatch(HooksCommands::Install, &ctx, &mut out, &mut Vec::new()).unwrap();

        assert_eq!(status, ExitCode::Success);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!(
                "Installed hooks to: {}\n\nInstalled:\n  pre-commit\n\nHooks are now active. Leases are enforced when a mutation is written, not at commit.\n",
                temp.path().join("hooks").display()
            )
        );

        let mut skipped_out = Vec::new();
        let status = dispatch(
            HooksCommands::Install,
            &ctx,
            &mut skipped_out,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(status, ExitCode::Success);
        assert_eq!(
            String::from_utf8(skipped_out).unwrap(),
            format!(
                "Installed hooks to: {}\n\nSkipped (already exist):\n  - pre-commit (already exists)\n\nHooks are now active. Leases are enforced when a mutation is written, not at commit.\n",
                temp.path().join("hooks").display()
            )
        );
    }

    #[test]
    fn test_dispatch_install_failure_uses_same_classification_and_exit_for_both_modes() {
        let temp = TempDir::new().unwrap();
        let executor = executor(temp.path(), false);
        [false, true].into_iter().for_each(|json| {
            let ctx = CommandContext {
                executor: &executor,
                mode: OutputMode { json, quiet: false },
            };

            let error = dispatch(
                HooksCommands::Install,
                &ctx,
                &mut Vec::new(),
                &mut Vec::new(),
            )
            .unwrap_err();
            let classified = classify(&error);

            assert_eq!(classified.error.code, ErrorCode::InvalidArgument);
            assert_eq!(classified.exit_code(), ExitCode::InvalidArgument);
        });
    }

    #[test]
    fn test_registered_output_schema_hooks_install_names_its_response() {
        let schema = crate::schema::CommandSchema::generate();
        let output = schema
            .commands
            .get("hooks")
            .and_then(|command| command.subcommands.as_ref())
            .and_then(|subcommands| subcommands.get("install"))
            .and_then(|command| command.output.as_ref())
            .expect("hooks install has a registered output schema");

        assert_eq!(output.success, "HooksInstallResponse");
    }
}
