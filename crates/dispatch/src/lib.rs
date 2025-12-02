//! jit-dispatch orchestrator library
//!
//! Provides orchestration capabilities for coordinating agents that work on issues
//! tracked by the jit issue tracker.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Orchestrator configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// How often to poll for ready issues (seconds)
    pub poll_interval_secs: u64,

    /// List of available agents
    pub agents: Vec<AgentConfig>,
}

impl Config {
    /// Load configuration from TOML file
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config from {:?}", path))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config from {:?}", path))?;

        Ok(config)
    }
}

/// Configuration for a single agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique identifier for this agent
    pub id: String,

    /// Type of agent (e.g., "copilot", "ci", "human")
    #[serde(rename = "type")]
    pub agent_type: String,

    /// Maximum concurrent tasks this agent can handle
    pub max_concurrent: usize,

    /// Command to execute for this agent
    pub command: String,
}

/// Tracks agent capacity and assignments
pub struct AgentTracker {
    agents: Vec<AgentConfig>,
    assignments: HashMap<String, Vec<String>>, // agent_id -> issue_ids
}

impl AgentTracker {
    /// Create a new agent tracker
    pub fn new(agents: Vec<AgentConfig>) -> Self {
        Self {
            agents,
            assignments: HashMap::new(),
        }
    }

    /// Get list of agents with available capacity
    pub fn available_agents(&self) -> Vec<&AgentConfig> {
        self.agents
            .iter()
            .filter(|agent| {
                let assigned = self
                    .assignments
                    .get(&agent.id)
                    .map(|v| v.len())
                    .unwrap_or(0);
                assigned < agent.max_concurrent
            })
            .collect()
    }

    /// Assign work to an agent
    pub fn assign_work(&mut self, agent_id: &str, issue_id: &str) -> Result<()> {
        // Find agent
        let agent = self
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| anyhow::anyhow!("Agent {} not found", agent_id))?;

        // Check capacity
        let assigned = self.assignments.entry(agent_id.to_string()).or_default();
        if assigned.len() >= agent.max_concurrent {
            bail!(
                "Agent {} at capacity ({}/{})",
                agent_id,
                assigned.len(),
                agent.max_concurrent
            );
        }

        assigned.push(issue_id.to_string());
        Ok(())
    }
}

/// Represents a ready issue from jit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyIssue {
    pub id: String,
    pub title: String,
    pub priority: String,
}

/// Main orchestrator
pub struct Orchestrator {
    repo_path: PathBuf,
    agent_tracker: Option<AgentTracker>,
}

impl Orchestrator {
    /// Create a new orchestrator for a jit repository
    pub fn new(repo_path: &Path) -> Self {
        Self {
            repo_path: repo_path.to_path_buf(),
            agent_tracker: None,
        }
    }

    /// Create orchestrator with configuration
    pub fn with_config(repo_path: &Path, config: Config) -> Self {
        let agent_tracker = AgentTracker::new(config.agents);

        Self {
            repo_path: repo_path.to_path_buf(),
            agent_tracker: Some(agent_tracker),
        }
    }

    /// Query ready issues from jit
    pub fn query_ready_issues(&self) -> Result<Vec<ReadyIssue>> {
        let jit_binary = self.find_jit_binary()?;

        let output = Command::new(jit_binary)
            .args(["query", "ready", "--json"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to execute jit query ready")?;

        if !output.status.success() {
            bail!(
                "jit query ready failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("Failed to parse jit query output")?;

        let issues: Vec<ReadyIssue> = json["data"]["issues"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Expected 'issues' array in response"))?
            .iter()
            .map(|issue| ReadyIssue {
                id: issue["id"].as_str().unwrap_or("").to_string(),
                title: issue["title"].as_str().unwrap_or("").to_string(),
                priority: issue["priority"].as_str().unwrap_or("normal").to_string(),
            })
            .collect();

        Ok(issues)
    }

    /// Get next issue to assign (highest priority)
    pub fn next_issue_to_assign(&self) -> Result<ReadyIssue> {
        let mut issues = self.query_ready_issues()?;

        if issues.is_empty() {
            bail!("No ready issues available");
        }

        // Sort by priority: critical > high > normal > low
        issues.sort_by(|a, b| {
            let priority_order = |p: &str| match p {
                "critical" => 0,
                "high" => 1,
                "normal" => 2,
                "low" => 3,
                _ => 4,
            };

            priority_order(&a.priority).cmp(&priority_order(&b.priority))
        });

        Ok(issues[0].clone())
    }

    /// Claim an issue for an agent
    pub fn claim_issue_for_agent(&mut self, issue_id: &str, agent_id: &str) -> Result<()> {
        let jit_binary = self.find_jit_binary()?;

        let status = Command::new(jit_binary)
            .args(["issue", "claim", issue_id, agent_id])
            .current_dir(&self.repo_path)
            .status()
            .context("Failed to execute jit issue claim")?;

        if !status.success() {
            bail!("jit issue claim failed");
        }

        Ok(())
    }

    /// Run one dispatch cycle: assign ready issues to available agents
    pub fn run_dispatch_cycle(&mut self) -> Result<usize> {
        let ready_issues = self.query_ready_issues()?;

        // Sort issues by priority
        let mut sorted_issues = ready_issues;
        sorted_issues.sort_by(|a, b| {
            let priority_order = |p: &str| match p {
                "critical" => 0,
                "high" => 1,
                "normal" => 2,
                "low" => 3,
                _ => 4,
            };

            priority_order(&a.priority).cmp(&priority_order(&b.priority))
        });

        // Build list of assignments to make
        let mut assignments = Vec::new();

        {
            let agent_tracker = self
                .agent_tracker
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No agent tracker configured"))?;

            for issue in &sorted_issues {
                let available = agent_tracker.available_agents();

                if available.is_empty() {
                    break; // No more capacity
                }

                // Pick first available agent
                let agent_id = available[0].id.clone();
                assignments.push((issue.id.clone(), agent_id));
            }
        }

        // Execute assignments
        for (issue_id, agent_id) in &assignments {
            let agent_id_full = format!("agent:{}", agent_id);
            self.claim_issue_for_agent(issue_id, &agent_id_full)?;

            // Track assignment
            if let Some(tracker) = self.agent_tracker.as_mut() {
                tracker.assign_work(agent_id, issue_id)?;
            }
        }

        Ok(assignments.len())
    }

    fn find_jit_binary(&self) -> Result<PathBuf> {
        // Look for jit binary in workspace target directory
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let binary = Path::new(manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target/debug/jit");

        if binary.exists() {
            Ok(binary)
        } else {
            bail!("jit binary not found at {:?}", binary);
        }
    }
}
