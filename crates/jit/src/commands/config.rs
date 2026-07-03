//! Config-mutation command (`jit config set`).
//!
//! Owns the `jit config set` write end to end: target-file resolution (repo vs
//! user-global), TOML document read/mutate, typed validation of the incoming
//! value (notably `project.name` through [`ProjectName`], REQ-03 of the
//! multi-jit story: an invalid identity must never reach the file), and the
//! atomic write through the storage-layer primitive
//! ([`write_file_atomic`](crate::storage::atomic_write::write_file_atomic),
//! INV-ATOMIC-WRITES). The CLI layer only parses args and formats the returned
//! [`ConfigSetOutcome`]; no config persistence lives in `main.rs` (the layer
//! boundary in CLAUDE.md "Separation of Concerns").

use super::*;
use crate::config::ProjectName;
use std::path::PathBuf;

/// Result of a `jit config set` write, carrying what the CLI needs to print.
///
/// Returned by [`CommandExecutor::set_config`]. `file` is the config file that
/// was written and `scope` is its origin token (`"user"` for a global write,
/// `"repo"` otherwise), mirroring the tokens the `--json` payload reports.
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
    /// empty one), parses `value` into the expected type for the key, and writes
    /// the document back atomically. Typed fields are validated ON WRITE, not
    /// only at load: `project.name` is parsed through [`ProjectName`] so an
    /// invalid identity is rejected before the write (REQ-03) and the file is
    /// left untouched. Numeric (`*_secs`/`*_pct`/`max_*`) and boolean
    /// (`enable_*`/`require_*`/`auto_*`) keys are likewise parsed into their TOML
    /// types; any other key is stored as a string.
    pub fn set_config(&self, key: &str, value: &str, global: bool) -> Result<ConfigSetOutcome> {
        use std::fs;

        // Determine the target config file.
        let config_path = if global {
            let home =
                dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))?;
            let config_dir = home.join(".config/jit");
            fs::create_dir_all(&config_dir)?;
            config_dir.join("config.toml")
        } else {
            self.storage.root().join("config.toml")
        };

        // Load the existing config or start an empty document.
        let mut doc = if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            content
                .parse::<toml_edit::DocumentMut>()
                .map_err(|e| anyhow!("Failed to parse config: {}", e))?
        } else {
            toml_edit::DocumentMut::new()
        };

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

        // Write back atomically (temp file + rename, INV-ATOMIC-WRITES).
        crate::storage::atomic_write::write_file_atomic(&config_path, &doc.to_string())?;

        Ok(ConfigSetOutcome {
            key: key.to_string(),
            value: value.to_string(),
            file: config_path,
            scope: if global { "user" } else { "repo" },
        })
    }
}
