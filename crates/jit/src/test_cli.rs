//! Shared fixture for tests that spawn the `jit` binary.
//!
//! A [`JitCli`] owns a temporary repository and builds every `Command` through
//! one convention: the binary of the calling test target, working directory at
//! the repository, test mode on, and no inherited `JIT_*` variable.

use anyhow::{anyhow, bail, ensure, Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Prefix of every environment variable the product reads.
const PRODUCT_ENV_PREFIX: &str = "JIT_";

/// Git variables that redirect a command away from the repository under test.
const GIT_REDIRECTS: [&str; 3] = ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"];

/// Author identity of repositories made by [`init_git_repository`].
pub const GIT_AUTHOR_NAME: &str = "Test User";
pub const GIT_AUTHOR_EMAIL: &str = "test@example.com";

/// A temporary repository and the `jit` processes spawned over it.
pub struct JitCli {
    dir: TempDir,
}

impl JitCli {
    /// An empty directory with no `.jit/`.
    pub fn uninitialised() -> Result<Self> {
        Ok(Self {
            dir: TempDir::new().context("create temporary repository")?,
        })
    }

    /// A repository initialised by `jit init`.
    pub fn initialised() -> Result<Self> {
        let cli = Self::uninitialised()?;
        cli.run(["init"])?;
        Ok(cli)
    }

    /// A repository built by shared test support; the fixture removes it on drop.
    pub fn with_repository(dir: TempDir) -> Self {
        Self { dir }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Initialise git in the repository (see [`init_git_repository`]).
    pub fn init_git(&self) -> Result<&Self> {
        init_git_repository(self.root())?;
        Ok(self)
    }

    /// A `jit` command in the repository with `args` and no `JIT_*` variable.
    pub fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.command_with_env(args, std::iter::empty::<(&str, &str)>())
    }

    /// Like [`Self::command`], with `env` set for this process alone.
    pub fn command_with_env<I, S, K, V>(
        &self,
        args: I,
        env: impl IntoIterator<Item = (K, V)>,
    ) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new(jit_binary());
        command.current_dir(self.root()).args(args);
        inherited_product_vars().for_each(|name| {
            command.env_remove(name);
        });
        command.env("JIT_TEST_MODE", "1").envs(env);
        command
    }

    /// Run `jit args` and return its stdout; fails with the stderr on a nonzero exit.
    pub fn run<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let output = self.command(args).output().context("spawn jit")?;
        ensure!(
            output.status.success(),
            "jit failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Run `jit args --json` and parse stdout.
    pub fn run_json<I, S>(&self, args: I) -> Result<serde_json::Value>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let args: Vec<OsString> = args
            .into_iter()
            .map(|arg| arg.as_ref().to_os_string())
            .chain(std::iter::once("--json".into()))
            .collect();
        let stdout = self.run(args)?;
        serde_json::from_str(&stdout).context("parse jit --json output")
    }

    /// Run `jit issue create args --json` and return the new issue's id.
    pub fn create_issue<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let args = ["issue".into(), "create".into()]
            .into_iter()
            .chain(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        created_issue_id(&self.run_json(args)?)
    }
}

/// The id in the `--json` output of a command that created an issue.
pub fn created_issue_id(output: &serde_json::Value) -> Result<String> {
    output
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("no string `id` in {output}"))
}

/// Initialise git in `path` with a fixed author identity and unsigned commits,
/// ignoring the developer's global and system git configuration.
pub fn init_git_repository(path: &Path) -> Result<()> {
    let git = |args: &[&str]| -> Result<()> {
        let mut command = Command::new("git");
        command
            .current_dir(path)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null");
        GIT_REDIRECTS.iter().for_each(|name| {
            command.env_remove(name);
        });
        let output = command.output().context("spawn git")?;
        if !output.status.success() {
            bail!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    };
    git(&["init", "--initial-branch=main"])?;
    git(&["config", "user.name", GIT_AUTHOR_NAME])?;
    git(&["config", "user.email", GIT_AUTHOR_EMAIL])?;
    git(&["config", "commit.gpgsign", "false"])
}

fn inherited_product_vars() -> impl Iterator<Item = OsString> {
    std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| name.to_string_lossy().starts_with(PRODUCT_ENV_PREFIX))
}

/// The `jit` binary of the running test target: Cargo's per-target
/// `CARGO_BIN_EXE_jit`, else the binary beside the test executable's `deps/`.
fn jit_binary() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_jit")
        .map(PathBuf::from)
        .or_else(|| {
            let exe = std::env::current_exe().ok()?;
            Some(exe.parent()?.parent()?.join("jit"))
        })
        .expect("locate the jit binary")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::EnvGuard;
    use serde_json::json;

    #[test]
    fn test_command_removes_inherited_product_vars_and_keeps_overrides_per_process() {
        let mut env = EnvGuard::acquire();
        env.set("JIT_AGENT_ID", "agent:ambient")
            .set("JIT_DATA_DIR", "/ambient");
        let cli = JitCli::uninitialised().unwrap();

        let plain = cli.command(["status"]);
        let overridden = cli.command_with_env(["status"], [("JIT_AGENT_ID", "agent:given")]);
        let value = |command: &Command, name: &str| {
            command
                .get_envs()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.map(|v| v.to_string_lossy().into_owned()))
        };

        assert_eq!(value(&plain, "JIT_AGENT_ID"), Some(None));
        assert_eq!(value(&plain, "JIT_DATA_DIR"), Some(None));
        assert_eq!(
            value(&overridden, "JIT_AGENT_ID"),
            Some(Some("agent:given".into()))
        );
        assert_eq!(value(&plain, "JIT_AGENT_ID"), Some(None));
    }

    #[test]
    fn test_init_git_repository_commits_without_global_git_configuration() {
        let home = TempDir::new().unwrap();
        std::fs::write(
            home.path().join(".gitconfig"),
            "[user]\n\tname = Developer\n\temail = dev@example.org\n[commit]\n\tgpgsign = true\n",
        )
        .unwrap();
        let mut env = EnvGuard::acquire();
        env.set("HOME", home.path()).remove("GIT_CONFIG_GLOBAL");
        let repo = TempDir::new().unwrap();

        init_git_repository(repo.path()).unwrap();
        let output = Command::new("git")
            .current_dir(repo.path())
            .args(["commit", "--allow-empty", "-m", "c"])
            .output()
            .unwrap();
        let author = Command::new("git")
            .current_dir(repo.path())
            .args(["log", "-1", "--format=%an <%ae>"])
            .output()
            .unwrap();

        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&author.stdout).trim(),
            format!("{GIT_AUTHOR_NAME} <{GIT_AUTHOR_EMAIL}>")
        );
    }

    #[test]
    fn test_created_issue_id_reads_id_field_and_rejects_other_output() {
        assert_eq!(
            created_issue_id(&json!({"id": "abc", "x": 1})).unwrap(),
            "abc"
        );
        assert!(created_issue_id(&json!({"count": 0})).is_err());
        assert!(created_issue_id(&json!({"id": 7})).is_err());
    }
}
