//! Agent identity configuration and resolution.
//!
//! This module implements agent identity resolution with the following priority:
//! 1. `--agent-id` CLI flag (highest priority, explicit override)
//! 2. `JIT_AGENT_ID` environment variable (session-specific)
//! 3. `~/.config/jit/agent.toml` config file (persistent identity)
//! 4. Error (no default, must be explicitly configured)
//!
//! Agent IDs follow the format `{type}:{identifier}`, for example:
//! - `agent:copilot-1` - GitHub Copilot session 1
//! - `human:alice` - Human user Alice
//! - `ci:github-actions` - CI/CD pipeline

use crate::domain::{Assignee, AssigneeParseError};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};

/// Agent configuration from `~/.config/jit/agent.toml`.
///
/// Unknown keys and sections are rejected on load.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    /// Agent configuration section
    pub agent: AgentSection,
}

/// Agent identity section
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSection {
    /// Persistent agent identity (format: {type}:{identifier})
    pub id: String,
    /// When this identity was created (ISO 8601)
    pub created_at: String,
    /// Human-readable description
    pub description: String,
}

impl AgentConfig {
    /// Load agent configuration from `~/.config/jit/agent.toml`.
    ///
    /// # Errors
    ///
    /// See [`AgentConfig::load_from`].
    pub fn load() -> Result<Option<Self>> {
        Self::load_from(&Self::config_path()?)
    }

    /// Load agent configuration from `config_path`; an absent file is `None`.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, is malformed TOML, holds an
    /// unknown or missing key, or carries an invalid agent ID. The message
    /// itself names the parse cause, since the CLI prints only the outermost
    /// message.
    pub fn load_from(config_path: &Path) -> Result<Option<Self>> {
        if !config_path.exists() {
            return Ok(None);
        }

        let content =
            std::fs::read_to_string(config_path).context("Failed to read agent config file")?;

        // The summary names the offending key even where only the top-level
        // message is printed; the typed source keeps the parse-error class.
        let config: AgentConfig = toml::from_str(&content).map_err(|e| {
            let summary = format!("Failed to parse agent.toml: {}", e.message());
            anyhow::Error::new(e).context(summary)
        })?;

        // Validate agent ID format
        validate_agent_id(&config.agent.id)?;

        Ok(Some(config))
    }

    /// Get the path to the agent config file.
    fn config_path() -> Result<PathBuf> {
        let config_dir =
            dirs::config_dir().ok_or_else(|| anyhow!("Could not determine config directory"))?;

        Ok(config_dir.join("jit").join("agent.toml"))
    }
}

/// Resolve agent identity with priority: CLI flag > env var > config file > error.
///
/// # Errors
///
/// Returns an error if no agent identity is configured or if the format is invalid.
pub fn resolve_agent_id(cli_flag: Option<String>) -> Result<Assignee> {
    resolve_agent_id_with(cli_flag, |key| env::var(key).ok(), AgentConfig::load)
}

/// [`resolve_agent_id`] over an environment lookup and a configuration loader.
pub fn resolve_agent_id_with(
    cli_flag: Option<String>,
    env_lookup: impl Fn(&str) -> Option<String>,
    load_config: impl FnOnce() -> Result<Option<AgentConfig>>,
) -> Result<Assignee> {
    if let Some(id) = cli_flag.or_else(|| env_lookup("JIT_AGENT_ID")) {
        return validate_agent_id(&id);
    }

    if let Some(config) = load_config()? {
        return validate_agent_id(&config.agent.id);
    }

    bail!(
        "No agent identity configured.\n\
         \n\
         Set one of the following (priority order):\n\
         1. CLI flag: --agent-id agent:your-name\n\
         2. Environment: export JIT_AGENT_ID=agent:your-name\n\
         3. Config file: ~/.config/jit/agent.toml\n\
         \n\
         Format: {{type}}:{{identifier}}\n\
         Examples: agent:copilot-1, human:alice, ci:github-actions"
    )
}

/// Validate agent ID format: {type}:{identifier}
///
/// # Errors
///
/// Returns an error if the format is invalid.
fn validate_agent_id(id: &str) -> Result<Assignee> {
    // Reuse the single `{type}:{identifier}` parse path, mapping its errors onto
    // the agent-identity wording. Agent IDs are stricter than a bare assignee:
    // whitespace is rejected on top of the shared structural validation.
    let assignee = id.parse::<Assignee>().map_err(|e| match e {
        AssigneeParseError::EmptyKind(_) => anyhow!("Agent ID type cannot be empty: '{}'", id),
        AssigneeParseError::EmptyIdentifier(_) => {
            anyhow!("Agent ID identifier cannot be empty: '{}'", id)
        }
        AssigneeParseError::Empty | AssigneeParseError::MissingSeparator(_) => anyhow!(
            "Invalid agent ID format: '{}'\n\
             Expected format: {{type}}:{{identifier}}\n\
             Examples: agent:copilot-1, human:alice, ci:github-actions",
            id
        ),
    })?;

    // Type and identifier should not contain whitespace
    if assignee.kind().contains(char::is_whitespace)
        || assignee.identifier().contains(char::is_whitespace)
    {
        bail!("Agent ID cannot contain whitespace: '{}'", id);
    }

    Ok(assignee)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_agent_id_valid() {
        assert!(validate_agent_id("agent:copilot-1").is_ok());
        assert!(validate_agent_id("human:alice").is_ok());
        assert!(validate_agent_id("ci:github-actions").is_ok());
        assert!(validate_agent_id("agent:session-123").is_ok());
    }

    #[test]
    fn test_validate_agent_id_missing_colon() {
        let result = validate_agent_id("invalid");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("format"));
    }

    #[test]
    fn test_validate_agent_id_empty_type() {
        let result = validate_agent_id(":identifier");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("type cannot be empty"));
    }

    #[test]
    fn test_validate_agent_id_empty_identifier() {
        let result = validate_agent_id("agent:");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("identifier cannot be empty"));
    }

    #[test]
    fn test_validate_agent_id_with_whitespace() {
        assert!(validate_agent_id("agent :copilot").is_err());
        assert!(validate_agent_id("agent: copilot").is_err());
        assert!(validate_agent_id("agent:copilot ").is_err());
    }

    const VALID_TOML: &str = r#"
[agent]
id = "agent:copilot-1"
created_at = "2026-01-03T12:00:00Z"
description = "GitHub Copilot Workspace Session 1"
"#;

    #[test]
    fn test_parse_agent_config_reads_every_field() {
        let config: AgentConfig = toml::from_str(VALID_TOML).unwrap();
        assert_eq!(config.agent.id, "agent:copilot-1");
        assert_eq!(config.agent.created_at, "2026-01-03T12:00:00Z");
        assert_eq!(
            config.agent.description,
            "GitHub Copilot Workspace Session 1"
        );
    }

    #[test]
    fn test_parse_agent_config_missing_required_field_fails() {
        for missing in ["created_at", "description"] {
            let toml: String = VALID_TOML
                .lines()
                .filter(|l| !l.starts_with(missing))
                .collect::<Vec<_>>()
                .join("\n");
            let err = toml::from_str::<AgentConfig>(&toml)
                .unwrap_err()
                .to_string();
            assert!(err.contains(missing), "{missing} must be required: {err}");
        }
    }

    #[test]
    fn test_load_from_unknown_key_error_names_the_key() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("agent.toml");
        for (extra, key) in [
            ("\n[behavior]\nauto_heartbeat = true\n", "behavior"),
            ("\n[other]\nx = 1\n", "other"),
            ("default_ttl_secs = 900\n", "default_ttl_secs"),
        ] {
            std::fs::write(&path, format!("{VALID_TOML}{extra}")).unwrap();
            let err = AgentConfig::load_from(&path).unwrap_err();
            // Rendered as the CLI prints a top-level error.
            let message = err.to_string();
            assert!(message.contains(key), "error must name {key}: {message}");
            assert_eq!(
                crate::output::failure::classify(&err).error.code,
                crate::output::ErrorCode::ParseError
            );
        }
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn config_with_id(id: &str) -> Result<Option<AgentConfig>> {
        Ok(Some(AgentConfig {
            agent: AgentSection {
                id: id.to_string(),
                created_at: "2026-01-03T12:00:00Z".to_string(),
                description: String::new(),
            },
        }))
    }

    #[test]
    fn test_resolve_agent_id_cli_flag_beats_env_and_config() {
        let env = |_: &str| Some("env:should-not-use".to_string());
        let result = resolve_agent_id_with(Some("agent:cli".to_string()), env, || {
            config_with_id("file:should-not-use")
        })
        .unwrap();
        assert_eq!(result.to_string(), "agent:cli");
    }

    #[test]
    fn test_resolve_agent_id_env_beats_config() {
        let env = |key: &str| (key == "JIT_AGENT_ID").then(|| "agent:from-env".to_string());
        let result =
            resolve_agent_id_with(None, env, || config_with_id("file:should-not-use")).unwrap();
        assert_eq!(result.to_string(), "agent:from-env");
    }

    #[test]
    fn test_resolve_agent_id_config_used_without_flag_or_env() {
        let result = resolve_agent_id_with(None, no_env, || config_with_id("agent:from-file"));
        assert_eq!(result.unwrap().to_string(), "agent:from-file");
    }

    #[test]
    fn test_resolve_agent_id_invalid_env_format_fails() {
        let env = |_: &str| Some("invalid-no-colon".to_string());
        let err = resolve_agent_id_with(None, env, || Ok(None))
            .unwrap_err()
            .to_string();
        assert!(err.contains("Invalid agent ID format"));
    }

    #[test]
    fn test_resolve_agent_id_loader_error_propagates() {
        let err = resolve_agent_id_with(None, no_env, || Err(anyhow!("unreadable")))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unreadable"));
    }

    #[test]
    fn test_resolve_agent_id_nothing_configured_names_every_source() {
        let err = resolve_agent_id_with(None, no_env, || Ok(None))
            .unwrap_err()
            .to_string();
        assert!(err.contains("No agent identity configured"));
        assert!(err.contains("--agent-id"));
        assert!(err.contains("JIT_AGENT_ID"));
        assert!(err.contains("agent.toml"));
    }
}
