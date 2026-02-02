//! Configuration file loading and parsing.
//!
//! JIT supports repository-level configuration through `.jit/config.toml`.
//! If no config file exists, the system falls back to sensible defaults.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Root configuration structure loaded from `.jit/config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct JitConfig {
    /// Schema version for migrations (optional).
    pub version: Option<VersionConfig>,
    /// Type hierarchy configuration (optional).
    pub type_hierarchy: Option<HierarchyConfigToml>,
    /// Validation behavior configuration (optional).
    pub validation: Option<ValidationConfig>,
    /// Documentation lifecycle configuration (optional).
    pub documentation: Option<DocumentationConfig>,
    /// Label namespace registry (optional - replaces labels.json).
    pub namespaces: Option<HashMap<String, NamespaceConfig>>,
    /// Worktree and parallel work configuration (optional).
    pub worktree: Option<WorktreeConfig>,
    /// Coordination settings for leases and agents (optional).
    pub coordination: Option<CoordinationConfig>,
    /// Global operations configuration (optional).
    pub global_operations: Option<GlobalOperationsConfig>,
    /// Lock file configuration (optional).
    pub locks: Option<LocksConfig>,
    /// Event logging configuration (optional).
    pub events: Option<EventsConfig>,
}

/// Schema version configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct VersionConfig {
    /// Schema version number (default: 1).
    pub schema: u32,
}

/// Type hierarchy configuration from TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct HierarchyConfigToml {
    /// Type name to hierarchy level mapping (lower = more strategic).
    pub types: HashMap<String, u8>,
    /// Type name to membership label namespace mapping (optional).
    pub label_associations: Option<HashMap<String, String>>,
    /// List of type names considered strategic (optional).
    pub strategic_types: Option<Vec<String>>,
}

/// Validation behavior configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ValidationConfig {
    /// Strictness level: "strict", "loose", or "permissive".
    pub strictness: Option<String>,
    /// Default type when none specified (optional).
    pub default_type: Option<String>,
    /// Require exactly one type:* label per issue (default: false).
    pub require_type_label: Option<bool>,
    /// Label format regex (optional).
    pub label_regex: Option<String>,
    /// Reject malformed labels (default: false).
    pub reject_malformed_labels: Option<bool>,
    /// Enforce namespace registry (default: false).
    pub enforce_namespace_registry: Option<bool>,
    /// Warn on orphaned leaf-level issues (default: true).
    pub warn_orphaned_leaves: Option<bool>,
    /// Warn on strategic issues without matching labels (default: true).
    pub warn_strategic_consistency: Option<bool>,
}

/// Documentation lifecycle management configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct DocumentationConfig {
    /// Root directory for development documentation (default: "dev").
    pub development_root: Option<String>,
    /// Paths subject to archival (default: ["dev/active", "dev/studies", "dev/sessions"]).
    pub managed_paths: Option<Vec<String>>,
    /// Where archived docs are stored (default: "dev/archive").
    pub archive_root: Option<String>,
    /// Paths that never archive (default: ["docs/"]).
    pub permanent_paths: Option<Vec<String>>,
    /// Archive category mappings (e.g., design -> features).
    pub categories: Option<HashMap<String, String>>,
}

impl DocumentationConfig {
    /// Get development root with default fallback.
    pub fn development_root(&self) -> String {
        self.development_root
            .clone()
            .unwrap_or_else(|| "dev".to_string())
    }

    /// Get managed paths with default fallback.
    pub fn managed_paths(&self) -> Vec<String> {
        self.managed_paths.clone().unwrap_or_else(|| {
            vec![
                "dev/active".to_string(),
                "dev/studies".to_string(),
                "dev/sessions".to_string(),
            ]
        })
    }

    /// Get archive root with default fallback.
    pub fn archive_root(&self) -> String {
        self.archive_root
            .clone()
            .unwrap_or_else(|| "dev/archive".to_string())
    }

    /// Get permanent paths with default fallback.
    pub fn permanent_paths(&self) -> Vec<String> {
        self.permanent_paths
            .clone()
            .unwrap_or_else(|| vec!["docs/".to_string()])
    }

    /// Get category mapping (key: category ID, value: archive subdirectory).
    pub fn categories(&self) -> HashMap<String, String> {
        self.categories.clone().unwrap_or_default()
    }
}

/// Label namespace configuration from TOML.
/// Replaces the namespace definitions in labels.json.
#[derive(Debug, Clone, Deserialize)]
pub struct NamespaceConfig {
    /// Human-readable description.
    pub description: String,
    /// Whether only one label from this namespace can be applied per issue.
    pub unique: bool,
    /// Example labels (optional, for documentation).
    pub examples: Option<Vec<String>>,
}

/// Worktree and parallel work configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct WorktreeConfig {
    /// Worktree mode: "auto", "on", or "off" (default: "auto").
    pub mode: Option<String>,
    /// Lease enforcement mode: "strict", "warn", or "off" (default: "strict").
    pub enforce_leases: Option<String>,
}

/// Worktree detection mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeMode {
    /// Detect git worktree and enable automatically (default).
    Auto,
    /// Force worktree mode (fail if not in worktree).
    On,
    /// Disable worktree features (use legacy .jit/ only).
    Off,
}

/// Enforcement mode for lease requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnforcementMode {
    /// Block operations without active lease (production-safe default).
    Strict,
    /// Warn but allow operations without lease (development-friendly).
    Warn,
    /// No enforcement - bypass lease checks (backward compatible).
    Off,
}

impl WorktreeConfig {
    /// Get the worktree mode, defaulting to Auto if not specified.
    pub fn worktree_mode(&self) -> Result<WorktreeMode> {
        match self.mode.as_deref() {
            None | Some("auto") => Ok(WorktreeMode::Auto),
            Some("on") => Ok(WorktreeMode::On),
            Some("off") => Ok(WorktreeMode::Off),
            Some(invalid) => anyhow::bail!(
                "Invalid worktree mode: '{}'. Valid options: 'auto', 'on', 'off'",
                invalid
            ),
        }
    }

    /// Get the enforcement mode, defaulting to Strict if not specified.
    pub fn enforcement_mode(&self) -> Result<EnforcementMode> {
        match self.enforce_leases.as_deref() {
            None | Some("strict") => Ok(EnforcementMode::Strict),
            Some("warn") => Ok(EnforcementMode::Warn),
            Some("off") => Ok(EnforcementMode::Off),
            Some(invalid) => anyhow::bail!(
                "Invalid enforce_leases mode: '{}'. Valid options: 'strict', 'warn', 'off'",
                invalid
            ),
        }
    }
}

/// Coordination settings for leases and multi-agent work.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CoordinationConfig {
    /// Default TTL for new leases in seconds (default: 600).
    pub default_ttl_secs: Option<u64>,
    /// Heartbeat interval for automatic lease renewal in seconds (default: 30).
    pub heartbeat_interval_secs: Option<u64>,
    /// Warn when lease has less than this percentage of TTL remaining (default: 10).
    pub lease_renewal_threshold_pct: Option<u8>,
    /// Staleness threshold for TTL=0 leases in seconds (default: 3600).
    pub stale_threshold_secs: Option<u64>,
    /// Maximum concurrent TTL=0 leases per agent (default: 2).
    pub max_indefinite_leases_per_agent: Option<u32>,
    /// Maximum concurrent TTL=0 leases per repository (default: 10).
    pub max_indefinite_leases_per_repo: Option<u32>,
    /// Automatic lease renewal by heartbeat daemon (default: false).
    pub auto_renew_leases: Option<bool>,
}

impl CoordinationConfig {
    pub fn default_ttl_secs(&self) -> u64 {
        self.default_ttl_secs.unwrap_or(600)
    }

    pub fn heartbeat_interval_secs(&self) -> u64 {
        self.heartbeat_interval_secs.unwrap_or(30)
    }

    pub fn lease_renewal_threshold_pct(&self) -> u8 {
        self.lease_renewal_threshold_pct.unwrap_or(10)
    }

    pub fn stale_threshold_secs(&self) -> u64 {
        self.stale_threshold_secs.unwrap_or(3600)
    }

    pub fn max_indefinite_leases_per_agent(&self) -> u32 {
        self.max_indefinite_leases_per_agent.unwrap_or(2)
    }

    pub fn max_indefinite_leases_per_repo(&self) -> u32 {
        self.max_indefinite_leases_per_repo.unwrap_or(10)
    }

    pub fn auto_renew_leases(&self) -> bool {
        self.auto_renew_leases.unwrap_or(false)
    }
}

/// Global operations configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GlobalOperationsConfig {
    /// Require common history with main for global operations (default: true).
    pub require_main_history: Option<bool>,
    /// Branches allowed to modify global config (default: ["main"]).
    pub allowed_branches: Option<Vec<String>>,
}

impl GlobalOperationsConfig {
    pub fn require_main_history(&self) -> bool {
        self.require_main_history.unwrap_or(true)
    }

    pub fn allowed_branches(&self) -> Vec<String> {
        self.allowed_branches
            .clone()
            .unwrap_or_else(|| vec!["main".to_string()])
    }
}

/// Lock file configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct LocksConfig {
    /// Maximum age for lock files before considered stale in seconds (default: 3600).
    pub max_age_secs: Option<u64>,
    /// Enable lock metadata for diagnostics (default: true).
    pub enable_metadata: Option<bool>,
}

impl LocksConfig {
    pub fn max_age_secs(&self) -> u64 {
        self.max_age_secs.unwrap_or(3600)
    }

    pub fn enable_metadata(&self) -> bool {
        self.enable_metadata.unwrap_or(true)
    }
}

/// Event logging configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct EventsConfig {
    /// Enable sequence numbers in event logs (default: true).
    pub enable_sequences: Option<bool>,
    /// Standardize event envelopes across control and data plane (default: true).
    pub use_unified_envelope: Option<bool>,
}

impl EventsConfig {
    pub fn enable_sequences(&self) -> bool {
        self.enable_sequences.unwrap_or(true)
    }

    pub fn use_unified_envelope(&self) -> bool {
        self.use_unified_envelope.unwrap_or(true)
    }
}

// ============================================================
// Agent Configuration (separate from repository config)
// ============================================================

/// Agent configuration loaded from `~/.config/jit/agent.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    /// Agent identity section.
    pub agent: AgentIdentity,
    /// Agent behavior section (optional).
    pub behavior: Option<AgentBehavior>,
}

/// Agent identity configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentIdentity {
    /// Persistent agent identity (format: type:identifier, e.g., "agent:copilot-1").
    pub id: String,
    /// When this identity was created (ISO 8601 timestamp).
    pub created_at: Option<String>,
    /// Human-readable description.
    pub description: Option<String>,
    /// Default TTL preference in seconds.
    pub default_ttl_secs: Option<u64>,
}

impl AgentIdentity {
    /// Get the default TTL, falling back to coordination default (600s).
    pub fn default_ttl_secs(&self) -> u64 {
        self.default_ttl_secs.unwrap_or(600)
    }
}

/// Agent behavior configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AgentBehavior {
    /// Auto-start heartbeat daemon for lease renewal (default: false).
    pub auto_heartbeat: Option<bool>,
    /// Heartbeat interval in seconds (default: 30).
    pub heartbeat_interval: Option<u64>,
}

impl AgentBehavior {
    pub fn auto_heartbeat(&self) -> bool {
        self.auto_heartbeat.unwrap_or(false)
    }

    pub fn heartbeat_interval(&self) -> u64 {
        self.heartbeat_interval.unwrap_or(30)
    }
}

impl AgentConfig {
    /// Load agent configuration from `agent.toml` in the given directory.
    ///
    /// Returns `Ok(None)` if the file doesn't exist.
    /// Returns an error if the file exists but is malformed.
    pub fn load(config_dir: &Path) -> Result<Option<Self>> {
        let config_path = config_dir.join("agent.toml");

        if !config_path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&config_path).context("Failed to read agent.toml")?;

        let config: AgentConfig = toml::from_str(&content).context("Failed to parse agent.toml")?;

        Ok(Some(config))
    }
}

impl JitConfig {
    /// Load configuration from `.jit/config.toml` if it exists.
    ///
    /// Returns an empty config (all fields None) if the file doesn't exist.
    /// Returns an error if the file exists but is malformed.
    pub fn load(jit_root: &Path) -> Result<Self> {
        let config_path = jit_root.join("config.toml");

        if !config_path.exists() {
            // No config file - return empty config (will use defaults)
            return Ok(JitConfig {
                version: None,
                type_hierarchy: None,
                validation: None,
                documentation: None,
                namespaces: None,
                worktree: None,
                coordination: None,
                global_operations: None,
                locks: None,
                events: None,
            });
        }

        let content =
            std::fs::read_to_string(&config_path).context("Failed to read config.toml")?;

        let config: JitConfig = toml::from_str(&content).context("Failed to parse config.toml")?;

        Ok(config)
    }
}

// ============================================================
// Config Loader with Priority and Merging
// ============================================================

/// Builder for loading configuration from multiple sources with priority.
///
/// Priority order (highest to lowest):
/// 1. Repository config (`.jit/config.toml`)
/// 2. User config (`~/.config/jit/config.toml`)
/// 3. System config (`/etc/jit/config.toml`)
/// 4. Defaults (hardcoded)
#[derive(Debug, Default)]
pub struct ConfigLoader {
    system_config: Option<JitConfig>,
    user_config: Option<JitConfig>,
    repo_config: Option<JitConfig>,
}

impl ConfigLoader {
    /// Create a new config loader with only defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load and add system-level config (`/etc/jit/config.toml`).
    pub fn with_system_config(mut self, config_dir: &Path) -> Result<Self> {
        self.system_config = Some(JitConfig::load(config_dir)?);
        Ok(self)
    }

    /// Load and add user-level config (`~/.config/jit/config.toml`).
    pub fn with_user_config(mut self, config_dir: &Path) -> Result<Self> {
        self.user_config = Some(JitConfig::load(config_dir)?);
        Ok(self)
    }

    /// Load and add repository-level config (`.jit/config.toml`).
    pub fn with_repo_config(mut self, jit_root: &Path) -> Result<Self> {
        self.repo_config = Some(JitConfig::load(jit_root)?);
        Ok(self)
    }

    /// Build the effective configuration by merging all sources.
    pub fn build(self) -> EffectiveConfig {
        EffectiveConfig {
            system_config: self.system_config,
            user_config: self.user_config,
            repo_config: self.repo_config,
        }
    }
}

/// Merged configuration from all sources with priority resolution.
///
/// When accessing a config value, checks sources in order:
/// repo > user > system > default
#[derive(Debug, Default)]
pub struct EffectiveConfig {
    system_config: Option<JitConfig>,
    user_config: Option<JitConfig>,
    repo_config: Option<JitConfig>,
}

impl EffectiveConfig {
    /// Get the effective worktree mode.
    /// Priority: env var > repo > user > system > default
    pub fn worktree_mode(&self) -> Result<WorktreeMode> {
        // Check env var first (highest priority)
        if let Ok(val) = std::env::var("JIT_WORKTREE_MODE") {
            return match val.to_lowercase().as_str() {
                "auto" => Ok(WorktreeMode::Auto),
                "on" => Ok(WorktreeMode::On),
                "off" => Ok(WorktreeMode::Off),
                invalid => anyhow::bail!(
                    "Invalid JIT_WORKTREE_MODE: '{}'. Valid options: 'auto', 'on', 'off'",
                    invalid
                ),
            };
        }

        // Check repo first, then user, then system
        if let Some(ref cfg) = self.repo_config {
            if let Some(ref wt) = cfg.worktree {
                if wt.mode.is_some() {
                    return wt.worktree_mode();
                }
            }
        }
        if let Some(ref cfg) = self.user_config {
            if let Some(ref wt) = cfg.worktree {
                if wt.mode.is_some() {
                    return wt.worktree_mode();
                }
            }
        }
        if let Some(ref cfg) = self.system_config {
            if let Some(ref wt) = cfg.worktree {
                if wt.mode.is_some() {
                    return wt.worktree_mode();
                }
            }
        }
        // Default
        Ok(WorktreeMode::Auto)
    }

    /// Get the effective enforcement mode.
    /// Priority: env var > repo > user > system > default
    pub fn enforcement_mode(&self) -> Result<EnforcementMode> {
        // Check env var first (highest priority)
        if let Ok(val) = std::env::var("JIT_ENFORCE_LEASES") {
            return match val.to_lowercase().as_str() {
                "strict" => Ok(EnforcementMode::Strict),
                "warn" => Ok(EnforcementMode::Warn),
                "off" => Ok(EnforcementMode::Off),
                invalid => anyhow::bail!(
                    "Invalid JIT_ENFORCE_LEASES: '{}'. Valid options: 'strict', 'warn', 'off'",
                    invalid
                ),
            };
        }

        if let Some(ref cfg) = self.repo_config {
            if let Some(ref wt) = cfg.worktree {
                if wt.enforce_leases.is_some() {
                    return wt.enforcement_mode();
                }
            }
        }
        if let Some(ref cfg) = self.user_config {
            if let Some(ref wt) = cfg.worktree {
                if wt.enforce_leases.is_some() {
                    return wt.enforcement_mode();
                }
            }
        }
        if let Some(ref cfg) = self.system_config {
            if let Some(ref wt) = cfg.worktree {
                if wt.enforce_leases.is_some() {
                    return wt.enforcement_mode();
                }
            }
        }
        Ok(EnforcementMode::Strict)
    }

    /// Get the effective agent ID from environment variable.
    /// Returns None if JIT_AGENT_ID is not set.
    pub fn agent_id(&self) -> Option<String> {
        std::env::var("JIT_AGENT_ID").ok()
    }

    /// Get effective coordination config with merged values.
    pub fn coordination(&self) -> MergedCoordinationConfig {
        MergedCoordinationConfig {
            repo: self
                .repo_config
                .as_ref()
                .and_then(|c| c.coordination.clone()),
            user: self
                .user_config
                .as_ref()
                .and_then(|c| c.coordination.clone()),
            system: self
                .system_config
                .as_ref()
                .and_then(|c| c.coordination.clone()),
        }
    }

    /// Get effective global operations config with merged values.
    pub fn global_operations(&self) -> MergedGlobalOperationsConfig {
        MergedGlobalOperationsConfig {
            repo: self
                .repo_config
                .as_ref()
                .and_then(|c| c.global_operations.clone()),
            user: self
                .user_config
                .as_ref()
                .and_then(|c| c.global_operations.clone()),
            system: self
                .system_config
                .as_ref()
                .and_then(|c| c.global_operations.clone()),
        }
    }

    /// Get effective locks config with merged values.
    pub fn locks(&self) -> MergedLocksConfig {
        MergedLocksConfig {
            repo: self.repo_config.as_ref().and_then(|c| c.locks.clone()),
            user: self.user_config.as_ref().and_then(|c| c.locks.clone()),
            system: self.system_config.as_ref().and_then(|c| c.locks.clone()),
        }
    }

    /// Get effective events config with merged values.
    pub fn events(&self) -> MergedEventsConfig {
        MergedEventsConfig {
            repo: self.repo_config.as_ref().and_then(|c| c.events.clone()),
            user: self.user_config.as_ref().and_then(|c| c.events.clone()),
            system: self.system_config.as_ref().and_then(|c| c.events.clone()),
        }
    }
}

/// Merged coordination config with priority resolution per field.
#[derive(Debug)]
pub struct MergedCoordinationConfig {
    repo: Option<CoordinationConfig>,
    user: Option<CoordinationConfig>,
    system: Option<CoordinationConfig>,
}

impl MergedCoordinationConfig {
    pub fn default_ttl_secs(&self) -> u64 {
        self.repo
            .as_ref()
            .and_then(|c| c.default_ttl_secs)
            .or_else(|| self.user.as_ref().and_then(|c| c.default_ttl_secs))
            .or_else(|| self.system.as_ref().and_then(|c| c.default_ttl_secs))
            .unwrap_or(600)
    }

    pub fn heartbeat_interval_secs(&self) -> u64 {
        self.repo
            .as_ref()
            .and_then(|c| c.heartbeat_interval_secs)
            .or_else(|| self.user.as_ref().and_then(|c| c.heartbeat_interval_secs))
            .or_else(|| self.system.as_ref().and_then(|c| c.heartbeat_interval_secs))
            .unwrap_or(30)
    }

    pub fn lease_renewal_threshold_pct(&self) -> u8 {
        self.repo
            .as_ref()
            .and_then(|c| c.lease_renewal_threshold_pct)
            .or_else(|| {
                self.user
                    .as_ref()
                    .and_then(|c| c.lease_renewal_threshold_pct)
            })
            .or_else(|| {
                self.system
                    .as_ref()
                    .and_then(|c| c.lease_renewal_threshold_pct)
            })
            .unwrap_or(10)
    }

    pub fn stale_threshold_secs(&self) -> u64 {
        self.repo
            .as_ref()
            .and_then(|c| c.stale_threshold_secs)
            .or_else(|| self.user.as_ref().and_then(|c| c.stale_threshold_secs))
            .or_else(|| self.system.as_ref().and_then(|c| c.stale_threshold_secs))
            .unwrap_or(3600)
    }

    pub fn max_indefinite_leases_per_agent(&self) -> u32 {
        self.repo
            .as_ref()
            .and_then(|c| c.max_indefinite_leases_per_agent)
            .or_else(|| {
                self.user
                    .as_ref()
                    .and_then(|c| c.max_indefinite_leases_per_agent)
            })
            .or_else(|| {
                self.system
                    .as_ref()
                    .and_then(|c| c.max_indefinite_leases_per_agent)
            })
            .unwrap_or(2)
    }

    pub fn max_indefinite_leases_per_repo(&self) -> u32 {
        self.repo
            .as_ref()
            .and_then(|c| c.max_indefinite_leases_per_repo)
            .or_else(|| {
                self.user
                    .as_ref()
                    .and_then(|c| c.max_indefinite_leases_per_repo)
            })
            .or_else(|| {
                self.system
                    .as_ref()
                    .and_then(|c| c.max_indefinite_leases_per_repo)
            })
            .unwrap_or(10)
    }

    pub fn auto_renew_leases(&self) -> bool {
        self.repo
            .as_ref()
            .and_then(|c| c.auto_renew_leases)
            .or_else(|| self.user.as_ref().and_then(|c| c.auto_renew_leases))
            .or_else(|| self.system.as_ref().and_then(|c| c.auto_renew_leases))
            .unwrap_or(false)
    }
}

/// Merged global operations config with priority resolution per field.
#[derive(Debug)]
pub struct MergedGlobalOperationsConfig {
    repo: Option<GlobalOperationsConfig>,
    user: Option<GlobalOperationsConfig>,
    system: Option<GlobalOperationsConfig>,
}

impl MergedGlobalOperationsConfig {
    pub fn require_main_history(&self) -> bool {
        self.repo
            .as_ref()
            .and_then(|c| c.require_main_history)
            .or_else(|| self.user.as_ref().and_then(|c| c.require_main_history))
            .or_else(|| self.system.as_ref().and_then(|c| c.require_main_history))
            .unwrap_or(true)
    }

    pub fn allowed_branches(&self) -> Vec<String> {
        self.repo
            .as_ref()
            .and_then(|c| c.allowed_branches.clone())
            .or_else(|| self.user.as_ref().and_then(|c| c.allowed_branches.clone()))
            .or_else(|| {
                self.system
                    .as_ref()
                    .and_then(|c| c.allowed_branches.clone())
            })
            .unwrap_or_else(|| vec!["main".to_string()])
    }
}

/// Merged locks config with priority resolution per field.
#[derive(Debug)]
pub struct MergedLocksConfig {
    repo: Option<LocksConfig>,
    user: Option<LocksConfig>,
    system: Option<LocksConfig>,
}

impl MergedLocksConfig {
    pub fn max_age_secs(&self) -> u64 {
        self.repo
            .as_ref()
            .and_then(|c| c.max_age_secs)
            .or_else(|| self.user.as_ref().and_then(|c| c.max_age_secs))
            .or_else(|| self.system.as_ref().and_then(|c| c.max_age_secs))
            .unwrap_or(3600)
    }

    pub fn enable_metadata(&self) -> bool {
        self.repo
            .as_ref()
            .and_then(|c| c.enable_metadata)
            .or_else(|| self.user.as_ref().and_then(|c| c.enable_metadata))
            .or_else(|| self.system.as_ref().and_then(|c| c.enable_metadata))
            .unwrap_or(true)
    }
}

/// Merged events config with priority resolution per field.
#[derive(Debug)]
pub struct MergedEventsConfig {
    repo: Option<EventsConfig>,
    user: Option<EventsConfig>,
    system: Option<EventsConfig>,
}

impl MergedEventsConfig {
    pub fn enable_sequences(&self) -> bool {
        self.repo
            .as_ref()
            .and_then(|c| c.enable_sequences)
            .or_else(|| self.user.as_ref().and_then(|c| c.enable_sequences))
            .or_else(|| self.system.as_ref().and_then(|c| c.enable_sequences))
            .unwrap_or(true)
    }

    pub fn use_unified_envelope(&self) -> bool {
        self.repo
            .as_ref()
            .and_then(|c| c.use_unified_envelope)
            .or_else(|| self.user.as_ref().and_then(|c| c.use_unified_envelope))
            .or_else(|| self.system.as_ref().and_then(|c| c.use_unified_envelope))
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_minimal_config() {
        let config_toml = r#"
[type_hierarchy]
types = { task = 1 }
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert!(config.type_hierarchy.is_some());
        assert!(config.validation.is_none());
    }

    #[test]
    fn test_parse_full_config() {
        let config_toml = r#"
[type_hierarchy]
types = { milestone = 1, epic = 2, task = 3 }

[type_hierarchy.label_associations]
epic = "epic"
milestone = "milestone"

[validation]
strictness = "loose"
warn_orphaned_leaves = true
warn_strategic_consistency = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        let hierarchy = config.type_hierarchy.unwrap();
        assert_eq!(hierarchy.types.len(), 3);
        assert_eq!(hierarchy.label_associations.as_ref().unwrap().len(), 2);

        let validation = config.validation.unwrap();
        assert_eq!(validation.strictness, Some("loose".to_string()));
        assert_eq!(validation.warn_orphaned_leaves, Some(true));
        assert_eq!(validation.warn_strategic_consistency, Some(false));
    }

    #[test]
    fn test_load_missing_config() {
        let temp_dir = TempDir::new().unwrap();
        let config = JitConfig::load(temp_dir.path()).unwrap();

        // Empty config when file doesn't exist
        assert!(config.type_hierarchy.is_none());
        assert!(config.validation.is_none());
    }

    #[test]
    fn test_load_existing_config() {
        let temp_dir = TempDir::new().unwrap();

        let config_toml = r#"
[type_hierarchy]
types = { epic = 1, task = 2 }
"#;
        std::fs::write(temp_dir.path().join("config.toml"), config_toml).unwrap();

        let config = JitConfig::load(temp_dir.path()).unwrap();
        assert!(config.type_hierarchy.is_some());
        assert_eq!(config.type_hierarchy.unwrap().types.len(), 2);
    }

    #[test]
    fn test_malformed_toml_returns_error() {
        let temp_dir = TempDir::new().unwrap();

        let bad_toml = "[broken syntax";
        std::fs::write(temp_dir.path().join("config.toml"), bad_toml).unwrap();

        let result = JitConfig::load(temp_dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_schema_v2_with_version() {
        let config_toml = r#"
[version]
schema = 2

[type_hierarchy]
types = { milestone = 1, epic = 2, task = 3 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
milestone = "milestone"
epic = "epic"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        assert!(config.version.is_some());
        assert_eq!(config.version.unwrap().schema, 2);

        let hierarchy = config.type_hierarchy.unwrap();
        assert_eq!(
            hierarchy.strategic_types,
            Some(vec!["milestone".to_string(), "epic".to_string()])
        );
    }

    #[test]
    fn test_parse_validation_with_new_fields() {
        let config_toml = r#"
[validation]
default_type = "task"
require_type_label = true
label_regex = '^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$'
reject_malformed_labels = true
enforce_namespace_registry = true
warn_orphaned_leaves = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        let validation = config.validation.unwrap();
        assert_eq!(validation.default_type, Some("task".to_string()));
        assert_eq!(validation.require_type_label, Some(true));
        assert_eq!(
            validation.label_regex,
            Some("^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$".to_string())
        );
        assert_eq!(validation.reject_malformed_labels, Some(true));
        assert_eq!(validation.enforce_namespace_registry, Some(true));
        assert_eq!(validation.warn_orphaned_leaves, Some(false));
    }

    #[test]
    fn test_parse_namespaces_from_toml() {
        let config_toml = r#"
[namespaces.type]
description = "Issue type (hierarchical)"
unique = true
examples = ["type:task", "type:epic"]

[namespaces.epic]
description = "Feature or initiative membership"
unique = false
examples = ["epic:auth", "epic:billing"]

[namespaces.component]
description = "Technical area"
unique = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        let namespaces = config.namespaces.unwrap();
        assert_eq!(namespaces.len(), 3);

        let type_ns = &namespaces["type"];
        assert_eq!(type_ns.description, "Issue type (hierarchical)");
        assert!(type_ns.unique);
        assert_eq!(
            type_ns.examples,
            Some(vec!["type:task".to_string(), "type:epic".to_string()])
        );

        let epic_ns = &namespaces["epic"];
        assert!(!epic_ns.unique);

        let component_ns = &namespaces["component"];
        assert!(component_ns.examples.is_none());
    }

    #[test]
    fn test_parse_full_schema_v2_config() {
        let config_toml = r#"
[version]
schema = 2

[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
milestone = "milestone"
epic = "epic"
story = "story"

[validation]
default_type = "task"
require_type_label = true
warn_orphaned_leaves = true
warn_strategic_consistency = true

[namespaces.type]
description = "Issue type"
unique = true

[namespaces.epic]
description = "Epic membership"
unique = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        // Version
        assert_eq!(config.version.unwrap().schema, 2);

        // Hierarchy
        let hierarchy = config.type_hierarchy.unwrap();
        assert_eq!(hierarchy.types.len(), 4);
        assert_eq!(
            hierarchy.strategic_types,
            Some(vec!["milestone".to_string(), "epic".to_string()])
        );

        // Validation
        let validation = config.validation.unwrap();
        assert_eq!(validation.default_type, Some("task".to_string()));
        assert_eq!(validation.require_type_label, Some(true));

        // Namespaces
        let namespaces = config.namespaces.unwrap();
        assert_eq!(namespaces.len(), 2);
        assert!(namespaces["type"].unique);
        assert!(!namespaces["epic"].unique);
    }

    #[test]
    fn test_enforcement_mode_default_to_strict() {
        let config_toml = r#"
[worktree]
# No enforce_leases specified
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(
            worktree.enforcement_mode().unwrap(),
            EnforcementMode::Strict
        );
    }

    #[test]
    fn test_enforcement_mode_explicit_strict() {
        let config_toml = r#"
[worktree]
enforce_leases = "strict"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(
            worktree.enforcement_mode().unwrap(),
            EnforcementMode::Strict
        );
    }

    #[test]
    fn test_enforcement_mode_warn() {
        let config_toml = r#"
[worktree]
enforce_leases = "warn"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.enforcement_mode().unwrap(), EnforcementMode::Warn);
    }

    #[test]
    fn test_enforcement_mode_off() {
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.enforcement_mode().unwrap(), EnforcementMode::Off);
    }

    #[test]
    fn test_enforcement_mode_invalid() {
        let config_toml = r#"
[worktree]
enforce_leases = "maybe"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        let result = worktree.enforcement_mode();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Invalid enforce_leases mode"));
        assert!(err_msg.contains("maybe"));
    }

    #[test]
    fn test_config_without_worktree_section() {
        let config_toml = r#"
[type_hierarchy]
types = { task = 1 }
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert!(config.worktree.is_none());
    }

    // ============================================================
    // Tests for new config sections (TDD - written before implementation)
    // ============================================================

    #[test]
    fn test_worktree_mode_auto() {
        let config_toml = r#"
[worktree]
mode = "auto"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.worktree_mode().unwrap(), WorktreeMode::Auto);
    }

    #[test]
    fn test_worktree_mode_on() {
        let config_toml = r#"
[worktree]
mode = "on"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.worktree_mode().unwrap(), WorktreeMode::On);
    }

    #[test]
    fn test_worktree_mode_off() {
        let config_toml = r#"
[worktree]
mode = "off"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.worktree_mode().unwrap(), WorktreeMode::Off);
    }

    #[test]
    fn test_worktree_mode_default_to_auto() {
        let config_toml = r#"
[worktree]
enforce_leases = "strict"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.worktree_mode().unwrap(), WorktreeMode::Auto);
    }

    #[test]
    fn test_worktree_mode_invalid() {
        let config_toml = r#"
[worktree]
mode = "maybe"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        let result = worktree.worktree_mode();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid worktree mode"));
    }

    #[test]
    fn test_coordination_config_full() {
        let config_toml = r#"
[coordination]
default_ttl_secs = 600
heartbeat_interval_secs = 30
lease_renewal_threshold_pct = 10
stale_threshold_secs = 3600
max_indefinite_leases_per_agent = 2
max_indefinite_leases_per_repo = 10
auto_renew_leases = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let coord = config.coordination.unwrap();
        assert_eq!(coord.default_ttl_secs, Some(600));
        assert_eq!(coord.heartbeat_interval_secs, Some(30));
        assert_eq!(coord.lease_renewal_threshold_pct, Some(10));
        assert_eq!(coord.stale_threshold_secs, Some(3600));
        assert_eq!(coord.max_indefinite_leases_per_agent, Some(2));
        assert_eq!(coord.max_indefinite_leases_per_repo, Some(10));
        assert_eq!(coord.auto_renew_leases, Some(false));
    }

    #[test]
    fn test_coordination_config_defaults() {
        let coord = CoordinationConfig::default();
        assert_eq!(coord.default_ttl_secs(), 600);
        assert_eq!(coord.heartbeat_interval_secs(), 30);
        assert_eq!(coord.lease_renewal_threshold_pct(), 10);
        assert_eq!(coord.stale_threshold_secs(), 3600);
        assert_eq!(coord.max_indefinite_leases_per_agent(), 2);
        assert_eq!(coord.max_indefinite_leases_per_repo(), 10);
        assert!(!coord.auto_renew_leases());
    }

    #[test]
    fn test_global_operations_config() {
        let config_toml = r#"
[global_operations]
require_main_history = true
allowed_branches = ["main", "develop"]
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let global_ops = config.global_operations.unwrap();
        assert_eq!(global_ops.require_main_history, Some(true));
        assert_eq!(
            global_ops.allowed_branches,
            Some(vec!["main".to_string(), "develop".to_string()])
        );
    }

    #[test]
    fn test_global_operations_defaults() {
        let global_ops = GlobalOperationsConfig::default();
        assert!(global_ops.require_main_history());
        assert_eq!(global_ops.allowed_branches(), vec!["main".to_string()]);
    }

    #[test]
    fn test_locks_config() {
        let config_toml = r#"
[locks]
max_age_secs = 7200
enable_metadata = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let locks = config.locks.unwrap();
        assert_eq!(locks.max_age_secs, Some(7200));
        assert_eq!(locks.enable_metadata, Some(false));
    }

    #[test]
    fn test_locks_config_defaults() {
        let locks = LocksConfig::default();
        assert_eq!(locks.max_age_secs(), 3600);
        assert!(locks.enable_metadata());
    }

    #[test]
    fn test_events_config() {
        let config_toml = r#"
[events]
enable_sequences = true
use_unified_envelope = true
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let events = config.events.unwrap();
        assert_eq!(events.enable_sequences, Some(true));
        assert_eq!(events.use_unified_envelope, Some(true));
    }

    #[test]
    fn test_events_config_defaults() {
        let events = EventsConfig::default();
        assert!(events.enable_sequences());
        assert!(events.use_unified_envelope());
    }

    #[test]
    fn test_full_parallel_work_config() {
        let config_toml = r#"
[worktree]
mode = "auto"
enforce_leases = "strict"

[coordination]
default_ttl_secs = 600
heartbeat_interval_secs = 30

[global_operations]
require_main_history = true
allowed_branches = ["main", "develop"]

[locks]
max_age_secs = 3600
enable_metadata = true

[events]
enable_sequences = true
use_unified_envelope = true
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        // All sections present
        assert!(config.worktree.is_some());
        assert!(config.coordination.is_some());
        assert!(config.global_operations.is_some());
        assert!(config.locks.is_some());
        assert!(config.events.is_some());

        // Verify worktree
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.worktree_mode().unwrap(), WorktreeMode::Auto);
        assert_eq!(
            worktree.enforcement_mode().unwrap(),
            EnforcementMode::Strict
        );

        // Verify coordination
        let coord = config.coordination.unwrap();
        assert_eq!(coord.default_ttl_secs(), 600);
    }

    // ============================================================
    // Agent configuration tests (TDD - written before implementation)
    // ============================================================

    #[test]
    fn test_agent_config_full() {
        let config_toml = r#"
[agent]
id = "agent:copilot-1"
created_at = "2026-01-03T12:00:00Z"
description = "GitHub Copilot Workspace Session 1"
default_ttl_secs = 900

[behavior]
auto_heartbeat = false
heartbeat_interval = 30
"#;
        let config: AgentConfig = toml::from_str(config_toml).unwrap();

        assert_eq!(config.agent.id, "agent:copilot-1");
        assert_eq!(
            config.agent.created_at,
            Some("2026-01-03T12:00:00Z".to_string())
        );
        assert_eq!(
            config.agent.description,
            Some("GitHub Copilot Workspace Session 1".to_string())
        );
        assert_eq!(config.agent.default_ttl_secs, Some(900));

        let behavior = config.behavior.unwrap();
        assert_eq!(behavior.auto_heartbeat, Some(false));
        assert_eq!(behavior.heartbeat_interval, Some(30));
    }

    #[test]
    fn test_agent_config_minimal() {
        let config_toml = r#"
[agent]
id = "agent:worker-1"
"#;
        let config: AgentConfig = toml::from_str(config_toml).unwrap();

        assert_eq!(config.agent.id, "agent:worker-1");
        assert!(config.agent.created_at.is_none());
        assert!(config.agent.description.is_none());
        assert!(config.agent.default_ttl_secs.is_none());
        assert!(config.behavior.is_none());
    }

    #[test]
    fn test_agent_identity_defaults() {
        let identity = AgentIdentity {
            id: "agent:test".to_string(),
            created_at: None,
            description: None,
            default_ttl_secs: None,
        };
        assert_eq!(identity.default_ttl_secs(), 600); // Default from coordination
    }

    #[test]
    fn test_agent_behavior_defaults() {
        let behavior = AgentBehavior::default();
        assert!(!behavior.auto_heartbeat());
        assert_eq!(behavior.heartbeat_interval(), 30);
    }

    #[test]
    fn test_agent_config_load_missing() {
        let temp_dir = TempDir::new().unwrap();
        let result = AgentConfig::load(temp_dir.path());
        // Should return None when file doesn't exist
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_agent_config_load_existing() {
        let temp_dir = TempDir::new().unwrap();
        let config_toml = r#"
[agent]
id = "agent:test-agent"
description = "Test agent"
"#;
        std::fs::write(temp_dir.path().join("agent.toml"), config_toml).unwrap();

        let config = AgentConfig::load(temp_dir.path()).unwrap().unwrap();
        assert_eq!(config.agent.id, "agent:test-agent");
        assert_eq!(config.agent.description, Some("Test agent".to_string()));
    }

    // ============================================================
    // Config loading with priority and merging tests (TDD)
    // ============================================================

    #[test]
    fn test_config_loader_defaults_only() {
        let loader = ConfigLoader::new();
        let config = loader.build();

        // Should have all defaults
        assert_eq!(config.coordination().default_ttl_secs(), 600);
        assert_eq!(config.coordination().heartbeat_interval_secs(), 30);
        assert!(config.global_operations().require_main_history());
        assert_eq!(config.locks().max_age_secs(), 3600);
        assert!(config.events().enable_sequences());
    }

    #[test]
    fn test_config_loader_repo_overrides_defaults() {
        let temp_dir = TempDir::new().unwrap();
        let config_toml = r#"
[coordination]
default_ttl_secs = 1200
"#;
        std::fs::write(temp_dir.path().join("config.toml"), config_toml).unwrap();

        let loader = ConfigLoader::new()
            .with_repo_config(temp_dir.path())
            .unwrap();
        let config = loader.build();

        // Repo value overrides default
        assert_eq!(config.coordination().default_ttl_secs(), 1200);
        // Other defaults preserved
        assert_eq!(config.coordination().heartbeat_interval_secs(), 30);
    }

    #[test]
    fn test_config_loader_repo_overrides_user() {
        let user_dir = TempDir::new().unwrap();
        let repo_dir = TempDir::new().unwrap();

        // User config sets TTL to 900
        let user_config = r#"
[coordination]
default_ttl_secs = 900
heartbeat_interval_secs = 60
"#;
        std::fs::write(user_dir.path().join("config.toml"), user_config).unwrap();

        // Repo config sets TTL to 1200 (overrides user)
        let repo_config = r#"
[coordination]
default_ttl_secs = 1200
"#;
        std::fs::write(repo_dir.path().join("config.toml"), repo_config).unwrap();

        let loader = ConfigLoader::new()
            .with_user_config(user_dir.path())
            .unwrap()
            .with_repo_config(repo_dir.path())
            .unwrap();
        let config = loader.build();

        // Repo overrides user for TTL
        assert_eq!(config.coordination().default_ttl_secs(), 1200);
        // User value used for heartbeat (not in repo config)
        assert_eq!(config.coordination().heartbeat_interval_secs(), 60);
    }

    #[test]
    fn test_config_loader_full_priority_chain() {
        let system_dir = TempDir::new().unwrap();
        let user_dir = TempDir::new().unwrap();
        let repo_dir = TempDir::new().unwrap();

        // System config (lowest priority after defaults)
        let system_config = r#"
[coordination]
default_ttl_secs = 300
heartbeat_interval_secs = 15
stale_threshold_secs = 1800
"#;
        std::fs::write(system_dir.path().join("config.toml"), system_config).unwrap();

        // User config overrides system
        let user_config = r#"
[coordination]
default_ttl_secs = 600
heartbeat_interval_secs = 30
"#;
        std::fs::write(user_dir.path().join("config.toml"), user_config).unwrap();

        // Repo config overrides user
        let repo_config = r#"
[coordination]
default_ttl_secs = 1200
"#;
        std::fs::write(repo_dir.path().join("config.toml"), repo_config).unwrap();

        let loader = ConfigLoader::new()
            .with_system_config(system_dir.path())
            .unwrap()
            .with_user_config(user_dir.path())
            .unwrap()
            .with_repo_config(repo_dir.path())
            .unwrap();
        let config = loader.build();

        // Repo wins for TTL
        assert_eq!(config.coordination().default_ttl_secs(), 1200);
        // User wins for heartbeat (not in repo)
        assert_eq!(config.coordination().heartbeat_interval_secs(), 30);
        // System wins for stale_threshold (not in user or repo)
        assert_eq!(config.coordination().stale_threshold_secs(), 1800);
    }

    #[test]
    fn test_config_loader_missing_files_ok() {
        let temp_dir = TempDir::new().unwrap();

        // Loading from non-existent paths should succeed (use defaults)
        let loader = ConfigLoader::new()
            .with_system_config(temp_dir.path())
            .unwrap()
            .with_user_config(temp_dir.path())
            .unwrap()
            .with_repo_config(temp_dir.path())
            .unwrap();
        let config = loader.build();

        // All defaults
        assert_eq!(config.coordination().default_ttl_secs(), 600);
    }

    #[test]
    fn test_effective_config_worktree_mode() {
        let temp_dir = TempDir::new().unwrap();
        let config_toml = r#"
[worktree]
mode = "on"
enforce_leases = "warn"
"#;
        std::fs::write(temp_dir.path().join("config.toml"), config_toml).unwrap();

        let loader = ConfigLoader::new()
            .with_repo_config(temp_dir.path())
            .unwrap();
        let config = loader.build();

        assert_eq!(config.worktree_mode().unwrap(), WorktreeMode::On);
        assert_eq!(config.enforcement_mode().unwrap(), EnforcementMode::Warn);
    }

    // ============================================================
    // Environment variable override tests (TDD)
    // ============================================================

    #[test]
    fn test_env_override_worktree_mode() {
        std::env::set_var("JIT_WORKTREE_MODE", "off");
        let config = ConfigLoader::new().build();
        assert_eq!(config.worktree_mode().unwrap(), WorktreeMode::Off);
        std::env::remove_var("JIT_WORKTREE_MODE");
    }

    #[test]
    fn test_env_override_enforce_leases() {
        std::env::set_var("JIT_ENFORCE_LEASES", "warn");
        let config = ConfigLoader::new().build();
        assert_eq!(config.enforcement_mode().unwrap(), EnforcementMode::Warn);
        std::env::remove_var("JIT_ENFORCE_LEASES");
    }

    #[test]
    fn test_env_override_agent_id() {
        std::env::set_var("JIT_AGENT_ID", "agent:env-test");
        let config = ConfigLoader::new().build();
        assert_eq!(config.agent_id(), Some("agent:env-test".to_string()));
        std::env::remove_var("JIT_AGENT_ID");
    }

    #[test]
    fn test_env_overrides_config_file() {
        let temp_dir = TempDir::new().unwrap();
        let config_toml = r#"
[worktree]
mode = "on"
enforce_leases = "strict"
"#;
        std::fs::write(temp_dir.path().join("config.toml"), config_toml).unwrap();

        // Env var should override config file
        std::env::set_var("JIT_WORKTREE_MODE", "off");
        let config = ConfigLoader::new()
            .with_repo_config(temp_dir.path())
            .unwrap()
            .build();
        assert_eq!(config.worktree_mode().unwrap(), WorktreeMode::Off);
        std::env::remove_var("JIT_WORKTREE_MODE");
    }

    #[test]
    fn test_env_invalid_value_returns_error() {
        std::env::set_var("JIT_WORKTREE_MODE", "invalid");
        let config = ConfigLoader::new().build();
        assert!(config.worktree_mode().is_err());
        std::env::remove_var("JIT_WORKTREE_MODE");
    }
}
