//! Git hooks installation commands

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

const PRE_COMMIT_HOOK: &str = include_str!("../../../../scripts/hooks/pre-commit");
const PRE_PUSH_HOOK: &str = include_str!("../../../../scripts/hooks/pre-push");

/// Install git hooks for lease and branch-drift validation
///
/// Copies hook templates into `<git_dir>/hooks/` and makes them executable.
/// `git_dir` is the common git directory of the selected repository.
///
/// # Hooks Installed
///
/// - **pre-commit**: Validates leases and branch drift before commit
/// - **pre-push**: Validates leases before push
///
/// # Errors
///
/// Returns error if the hooks directory cannot be written or permissions cannot be set.
pub fn install_hooks(git_dir: &Path) -> Result<InstallResult> {
    let hooks_dir = git_dir.join("hooks");

    // Create hooks directory if it doesn't exist
    if !hooks_dir.exists() {
        fs::create_dir_all(&hooks_dir).context("Failed to create .git/hooks directory")?;
    }

    let mut installed = Vec::new();
    let mut skipped = Vec::new();

    // Install pre-commit hook
    let pre_commit_path = hooks_dir.join("pre-commit");
    if pre_commit_path.exists() {
        skipped.push("pre-commit (already exists)".to_string());
    } else {
        fs::write(&pre_commit_path, PRE_COMMIT_HOOK).context("Failed to write pre-commit hook")?;
        set_executable(&pre_commit_path)?;
        installed.push("pre-commit".to_string());
    }

    // Install pre-push hook
    let pre_push_path = hooks_dir.join("pre-push");
    if pre_push_path.exists() {
        skipped.push("pre-push (already exists)".to_string());
    } else {
        fs::write(&pre_push_path, PRE_PUSH_HOOK).context("Failed to write pre-push hook")?;
        set_executable(&pre_push_path)?;
        installed.push("pre-push".to_string());
    }

    Ok(InstallResult {
        hooks_dir: hooks_dir.to_string_lossy().to_string(),
        installed,
        skipped,
    })
}

/// Result of hook installation
#[derive(Debug, serde::Serialize)]
pub struct InstallResult {
    pub hooks_dir: String,
    pub installed: Vec<String>,
    pub skipped: Vec<String>,
}

/// Set executable permissions on a file (Unix only)
#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = fs::metadata(path)
        .context("Failed to get file metadata")?
        .permissions();

    // Add execute permission for owner, group, and others
    let mode = perms.mode();
    perms.set_mode(mode | 0o111);

    fs::set_permissions(path, perms).context("Failed to set executable permission")?;

    Ok(())
}

/// Set executable permissions on a file (Windows - no-op)
#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    // Windows doesn't use Unix-style permissions
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_install_hooks() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir(&git_dir).unwrap();

        let result = install_hooks(&git_dir).unwrap();

        assert_eq!(result.installed.len(), 2);
        assert!(result.installed.contains(&"pre-commit".to_string()));
        assert!(result.installed.contains(&"pre-push".to_string()));
        assert!(result.skipped.is_empty());

        // Verify files exist
        assert!(git_dir.join("hooks/pre-commit").exists());
        assert!(git_dir.join("hooks/pre-push").exists());

        // Verify executable (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(git_dir.join("hooks/pre-commit"))
                .unwrap()
                .permissions();
            assert_ne!(perms.mode() & 0o111, 0);
        }
    }

    #[test]
    fn test_install_hooks_skips_existing() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        let hooks_dir = git_dir.join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        // Create existing pre-commit
        fs::write(hooks_dir.join("pre-commit"), "existing hook").unwrap();

        let result = install_hooks(&git_dir).unwrap();

        assert_eq!(result.installed.len(), 1);
        assert!(result.installed.contains(&"pre-push".to_string()));
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0].contains("pre-commit"));

        // Verify existing hook wasn't overwritten
        let content = fs::read_to_string(hooks_dir.join("pre-commit")).unwrap();
        assert_eq!(content, "existing hook");
    }

    const ISSUE_ID: &str = "514df7d3-caa6-40bb-9fba-d250e1bb756d";
    const AGENT: &str = "agent:holder";
    const STRICT_CONFIG: &str = "[worktree]\nenforce_leases = \"strict\"\n";

    fn git(repo: &Path, args: &[&str]) -> std::process::Output {
        std::process::Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args(args)
            .current_dir(repo)
            .env("JIT_AGENT_ID", AGENT)
            .output()
            .unwrap()
    }

    fn write(repo: &Path, path: &str, content: &str) {
        let full = repo.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
    }

    /// Strict-enforcement repository on a `feature` branch diverged from `main`,
    /// with the shipped hook installed.
    fn diverged_strict_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        let repo = temp.path();
        git(repo, &["init", "-q", "-b", "main"]);
        write(
            repo,
            ".jit/config.toml",
            "[worktree]\nenforce_leases = \"strict\"\n",
        );
        write(repo, ".jit/gates.toml", "");
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-q", "-m", "base"]);
        git(repo, &["checkout", "-q", "-b", "feature"]);
        git(repo, &["checkout", "-q", "main"]);
        write(repo, "main-only.txt", "x");
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-q", "-m", "main advances"]);
        git(repo, &["checkout", "-q", "feature"]);
        install_hooks(&repo.join(".git")).unwrap();
        temp
    }

    fn commit_succeeds(repo: &Path, path: &str, content: &str) -> bool {
        write(repo, path, content);
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-q", "-m", "change"])
            .status
            .success()
    }

    fn grant_lease(repo: &Path, holder: &str) {
        let index = serde_json::json!({ "leases": [{
            "lease_id": "00000000-0000-4000-8000-000000000001", "issue_id": ISSUE_ID, "agent_id": holder, "stale": false
        }] });
        write(repo, ".git/jit/claims.index.json", &index.to_string());
    }

    #[test]
    fn test_pre_commit_hook_blocks_global_files_on_diverged_branch() {
        for path in [".jit/gates.toml", ".jit/config.toml"] {
            let temp = diverged_strict_repo();
            assert!(
                !commit_succeeds(temp.path(), path, &format!("{STRICT_CONFIG}# edit\n")),
                "{path} commit must be blocked on a diverged branch"
            );
        }
    }

    #[test]
    fn test_pre_commit_hook_blocks_issue_file_without_lease_in_strict_mode() {
        let issue = format!(".jit/issues/{ISSUE_ID}.json");
        let no_index = diverged_strict_repo();
        assert!(!commit_succeeds(no_index.path(), &issue, "{}"));

        let other_holder = diverged_strict_repo();
        grant_lease(other_holder.path(), "agent:other");
        assert!(!commit_succeeds(other_holder.path(), &issue, "{}"));
    }

    #[test]
    fn test_pre_commit_hook_allows_issue_file_with_lease_held_by_committer() {
        let temp = diverged_strict_repo();
        grant_lease(temp.path(), AGENT);
        assert!(commit_succeeds(
            temp.path(),
            &format!(".jit/issues/{ISSUE_ID}.json"),
            "{}"
        ));
    }
}
