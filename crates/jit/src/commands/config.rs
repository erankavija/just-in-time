//! Config-mutation command (`jit config set`).
//!
//! Owns the `jit config set` command logic: target-file resolution (repo vs
//! user-global), key parsing, per-type value dispatch, and typed validation of
//! the incoming value (notably `project.name` through [`ProjectName`], REQ-03 of
//! the multi-jit story: an invalid identity must never reach the file). ALL
//! config-file IO — reading the TOML document and the atomic write — is
//! delegated to [`crate::storage::config_store`], so no persistence lives in
//! this command module (the layer boundary in CLAUDE.md "Separation of
//! Concerns"). The CLI layer only parses args and formats the returned
//! [`ConfigSetOutcome`].

use super::*;
use crate::config::ProjectName;
use crate::storage::config_store;
use std::path::PathBuf;

/// Result of a `jit config set` write, carrying what the CLI needs to print.
///
/// Returned by [`CommandExecutor::set_config`]. `file` is the config file that
/// was written and `scope` is its origin token (`"user"` for a global write,
/// `"repo"` otherwise), mirroring the tokens the `--json` payload reports.
///
/// # Examples
///
/// ```
/// use jit::commands::ConfigSetOutcome;
/// use std::path::PathBuf;
///
/// // The fields mirror a completed repo-scoped set (built by hand here to show
/// // the serialized shape).
/// let outcome = ConfigSetOutcome {
///     key: "project.name".to_string(),
///     value: "my-project".to_string(),
///     file: PathBuf::from(".jit/config.toml"),
///     scope: "repo",
/// };
/// assert_eq!(outcome.scope, "repo");
/// let json = serde_json::to_value(&outcome).unwrap();
/// assert_eq!(json["key"], "project.name");
/// assert_eq!(json["value"], "my-project");
/// ```
#[derive(Debug, Serialize)]
pub struct ConfigSetOutcome {
    /// The `section.field` key that was set.
    pub key: String,
    /// The value as written (the raw string the user supplied).
    pub value: String,
    /// The config file that was written.
    pub file: PathBuf,
    /// Config scope token: `"user"` for a global write, `"repo"` otherwise.
    pub scope: &'static str,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Set a `section.field` key in the repo (or, with `global`, the user-global)
    /// `config.toml`, returning what the CLI needs to print.
    ///
    /// Resolves the target file, reads the existing TOML document (or starts an
    /// empty one) and writes it back through [`crate::storage::config_store`] —
    /// this command owns no file IO. `value` is parsed into the expected type for
    /// the key BEFORE the write: `project.name` is validated through
    /// [`ProjectName`] so an invalid identity is rejected and the file is left
    /// untouched (REQ-03). Numeric (`*_secs`/`*_pct`/`max_*`) and boolean
    /// (`enable_*`/`require_*`/`auto_*`) keys are parsed into their TOML types;
    /// any other key is stored as a string.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let outcome = executor
    ///     .set_config("project.name", "renamed-project", false)
    ///     .unwrap();
    /// assert_eq!(outcome.scope, "repo");
    /// assert_eq!(outcome.value, "renamed-project");
    /// ```
    pub fn set_config(&self, key: &str, value: &str, global: bool) -> Result<ConfigSetOutcome> {
        // Determine the target config file (path derivation only; the store owns
        // the read/write IO).
        let config_path = if global {
            let home =
                dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))?;
            home.join(".config/jit").join("config.toml")
        } else {
            config_store::repo_config_path(self.storage.root())
        };

        // Load the existing config or start an empty document (through storage).
        let mut doc = config_store::read_config_document(&config_path)?;

        // Parse the key into section.field.
        let parts: Vec<&str> = key.split('.').collect();
        if parts.len() != 2 {
            anyhow::bail!("Config key must be in format 'section.field' (e.g., coordination.default_ttl_secs)");
        }
        let section = parts[0];
        let field = parts[1];

        // Ensure the section exists.
        if doc.get(section).is_none() {
            doc[section] = toml_edit::Item::Table(toml_edit::Table::new());
        }

        // Parse and set the value based on the expected type.
        let parsed_value: toml_edit::Item = match key {
            // Typed fields validate on write, not only at load: an invalid value
            // must never reach the file (REQ-03 of the multi-jit story: project
            // identity is write-validated).
            "project.name" => {
                let name: ProjectName = value.parse()?;
                toml_edit::value(name.as_str())
            }
            k if k.ends_with("_secs") || k.ends_with("_pct") || k.contains("max_") => {
                let num: i64 = value
                    .parse()
                    .map_err(|_| anyhow!("Expected numeric value for {}", key))?;
                toml_edit::value(num)
            }
            k if k.contains("enable_") || k.contains("require_") || k.contains("auto_") => {
                let b: bool = value
                    .parse()
                    .map_err(|_| anyhow!("Expected boolean (true/false) for {}", key))?;
                toml_edit::value(b)
            }
            _ => toml_edit::value(value),
        };

        doc[section][field] = parsed_value;

        // Persist through storage (atomic write, parent dir ensured for the
        // user-global first-write case).
        config_store::save_config_document(&config_path, &doc)?;

        Ok(ConfigSetOutcome {
            key: key.to_string(),
            value: value.to_string(),
            file: config_path,
            scope: if global { "user" } else { "repo" },
        })
    }
}
