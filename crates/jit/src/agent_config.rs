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

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;

/// Agent configuration from `~/.config/jit/agent.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentConfig {
    /// Agent configuration section
    pub agent: AgentSection,
    /// Behavioral preferences (optional)
    #[serde(default)]
    pub behavior: BehaviorSection,
}

/// Agent identity section
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSection {
    /// Persistent agent identity (format: {type}:{identifier})
    pub id: String,
    /// When this identity was created (ISO 8601)
    pub created_at: String,
    /// Human-readable description
    pub description: String,
    /// Optional default TTL preference (in seconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_ttl_secs: Option<u64>,
}

/// Behavioral preferences section
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BehaviorSection {
    /// Auto-start heartbeat daemon for lease renewal
    #[serde(default)]
    pub auto_heartbeat: bool,
    /// Heartbeat interval in seconds (default: 30)
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval: u64,
}

impl Default for BehaviorSection {
    fn default() -> Self {
        Self {
            auto_heartbeat: false,
            heartbeat_interval: 30,
        }
    }
}

fn default_heartbeat_interval() -> u64 {
    30
}

impl AgentConfig {
    /// Load agent configuration from `~/.config/jit/agent.toml`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Config file exists but cannot be read
    /// - Config file exists but is malformed TOML
    /// - Agent ID format is invalid
    pub fn load() -> Result<Option<Self>> {
        let config_path = Self::config_path()?;

        if !config_path.exists() {
            return Ok(None);
        }

        let content =
            std::fs::read_to_string(&config_path).context("Failed to read agent config file")?;

        let config: AgentConfig = toml::from_str(&content).context("Failed to parse agent.toml")?;

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
/// # Arguments
///
/// * `cli_flag` - Optional agent ID from CLI `--agent-id` flag
///
/// # Errors
///
/// Returns an error if no agent identity is configured or if the format is invalid.
///
/// # Examples
///
/// ```no_run
/// use jit::agent_config::resolve_agent_id;
///
/// // With CLI flag (highest priority)
/// let agent_id = resolve_agent_id(Some("agent:cli-override".to_string())).unwrap();
/// assert_eq!(agent_id, "agent:cli-override");
/// ```
pub fn resolve_agent_id(cli_flag: Option<String>) -> Result<String> {
    // Priority 1: CLI flag
    if let Some(id) = cli_flag {
        validate_agent_id(&id)?;
        return Ok(id);
    }

    // Priority 2: Environment variable
    if let Ok(id) = env::var("JIT_AGENT_ID") {
        validate_agent_id(&id)?;
        return Ok(id);
    }

    // Priority 3: Config file
    if let Some(config) = AgentConfig::load()? {
        return Ok(config.agent.id);
    }

    // No configuration found
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
    );
}

/// Validate agent ID format: {type}:{identifier}
///
/// # Errors
///
/// Returns an error if the format is invalid.
fn validate_agent_id(id: &str) -> Result<()> {
    if !id.contains(':') {
        bail!(
            "Invalid agent ID format: '{}'\n\
             Expected format: {{type}}:{{identifier}}\n\
             Examples: agent:copilot-1, human:alice, ci:github-actions",
            id
        );
    }

    let parts: Vec<&str> = id.splitn(2, ':').collect();
    if parts.len() != 2 {
        bail!("Invalid agent ID format: '{}'", id);
    }

    let (type_part, identifier_part) = (parts[0], parts[1]);

    if type_part.is_empty() {
        bail!("Agent ID type cannot be empty: '{}'", id);
    }

    if identifier_part.is_empty() {
        bail!("Agent ID identifier cannot be empty: '{}'", id);
    }

    // Type and identifier should not contain whitespace
    if type_part.contains(char::is_whitespace) || identifier_part.contains(char::is_whitespace) {
        bail!("Agent ID cannot contain whitespace: '{}'", id);
    }

    Ok(())
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

    #[test]
    fn test_parse_agent_config() {
        let toml = r#"
[agent]
id = "agent:copilot-1"
created_at = "2026-01-03T12:00:00Z"
description = "GitHub Copilot Workspace Session 1"
default_ttl_secs = 900

[behavior]
auto_heartbeat = false
heartbeat_interval = 30
"#;

        let config: AgentConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.agent.id, "agent:copilot-1");
        assert_eq!(config.agent.created_at, "2026-01-03T12:00:00Z");
        assert_eq!(
            config.agent.description,
            "GitHub Copilot Workspace Session 1"
        );
        assert_eq!(config.agent.default_ttl_secs, Some(900));
        assert!(!config.behavior.auto_heartbeat);
        assert_eq!(config.behavior.heartbeat_interval, 30);
    }

    #[test]
    fn test_parse_minimal_agent_config() {
        let toml = r#"
[agent]
id = "human:alice"
created_at = "2026-01-06T12:00:00Z"
description = "Alice's development machine"
"#;

        let config: AgentConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.agent.id, "human:alice");
        assert_eq!(config.agent.default_ttl_secs, None);
        assert!(!config.behavior.auto_heartbeat);
        assert_eq!(config.behavior.heartbeat_interval, 30); // default
    }

    #[test]
    fn test_resolve_agent_id_cli_flag_priority() {
        // Save current env state
        let original = env::var("JIT_AGENT_ID").ok();

        // CLI flag should take highest priority
        env::set_var("JIT_AGENT_ID", "env:should-not-use");

        let result = resolve_agent_id(Some("agent:cli-override".to_string())).unwrap();
        assert_eq!(result, "agent:cli-override");

        // Restore original env state
        match original {
            Some(val) => env::set_var("JIT_AGENT_ID", val),
            None => env::remove_var("JIT_AGENT_ID"),
        }
    }

    #[test]
    fn test_resolve_agent_id_env_var() {
        // Save current env state
        let original = env::var("JIT_AGENT_ID").ok();

        env::set_var("JIT_AGENT_ID", "agent:from-env");

        let result = resolve_agent_id(None).unwrap();
        assert_eq!(result, "agent:from-env");

        // Restore original env state
        match original {
            Some(val) => env::set_var("JIT_AGENT_ID", val),
            None => env::remove_var("JIT_AGENT_ID"),
        }
    }

    #[test]
    fn test_resolve_agent_id_invalid_format() {
        // Save current env state
        let original = env::var("JIT_AGENT_ID").ok();

        env::set_var("JIT_AGENT_ID", "invalid-no-colon");

        let result = resolve_agent_id(None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid agent ID format"));

        // Restore original env state
        match original {
            Some(val) => env::set_var("JIT_AGENT_ID", val),
            None => env::remove_var("JIT_AGENT_ID"),
        }
    }

    #[test]
    fn test_resolve_agent_id_no_config_error() {
        // Save current env state
        let original = env::var("JIT_AGENT_ID").ok();

        // Ensure no env var set
        env::remove_var("JIT_AGENT_ID");

        let result = resolve_agent_id(None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No agent identity configured"));
        assert!(err.contains("--agent-id"));
        assert!(err.contains("JIT_AGENT_ID"));
        assert!(err.contains("agent.toml"));

        // Restore original env state
        match original {
            Some(val) => env::set_var("JIT_AGENT_ID", val),
            None => env::remove_var("JIT_AGENT_ID"),
        }
    }

    #[test]
    fn test_behavior_section_defaults() {
        let toml = r#"
[agent]
id = "agent:test"
created_at = "2026-01-06T00:00:00Z"
description = "Test"
"#;

        let config: AgentConfig = toml::from_str(toml).unwrap();
        assert!(!config.behavior.auto_heartbeat);
        assert_eq!(config.behavior.heartbeat_interval, 30);
    }
}
