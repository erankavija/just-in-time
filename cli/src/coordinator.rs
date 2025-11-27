//! Coordinator daemon for agent orchestration.
//!
//! The coordinator watches for ready-to-work issues and dispatches them to agents
//! from the configured pool. It monitors agent health, logs events, and handles
//! work reassignment for stalled tasks.

use crate::domain::{Event, Issue, Priority, State};
use crate::storage::Storage;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

/// Configuration for a single agent in the pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique identifier for this agent
    pub id: String,
    /// Type of agent (e.g., "copilot", "ci", "human")
    pub agent_type: String,
    /// Command to execute to start the agent
    pub command: String,
    /// Maximum concurrent tasks this agent can handle
    pub max_concurrent: usize,
}

/// Dispatch rules for work assignment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchRules {
    /// Priority order for selecting issues
    pub priority_order: Vec<String>,
    /// Timeout in minutes before reassigning stalled work
    pub stall_timeout_minutes: u64,
}

impl Default for DispatchRules {
    fn default() -> Self {
        Self {
            priority_order: vec![
                "critical".to_string(),
                "high".to_string(),
                "normal".to_string(),
                "low".to_string(),
            ],
            stall_timeout_minutes: 30,
        }
    }
}

/// Coordinator configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorConfig {
    /// Pool of available agents
    pub agent_pool: Vec<AgentConfig>,
    /// Dispatch rules
    pub dispatch_rules: DispatchRules,
    /// Polling interval in seconds
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
}

fn default_poll_interval() -> u64 {
    5
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            agent_pool: Vec::new(),
            dispatch_rules: DispatchRules::default(),
            poll_interval_secs: 5,
        }
    }
}

/// Status of an active agent
#[derive(Debug, Clone)]
pub struct AgentStatus {
    pub agent_id: String,
    pub status: String,
    pub assigned_issue: Option<String>,
    pub started_at: Option<u64>,
}

/// The coordinator daemon
pub struct Coordinator {
    storage: Storage,
    config: CoordinatorConfig,
    running_agents: HashMap<String, Vec<Child>>,
}

impl Coordinator {
    /// Create a new coordinator with the given storage and config
    pub fn new(storage: Storage, config: CoordinatorConfig) -> Self {
        Self {
            storage,
            config,
            running_agents: HashMap::new(),
        }
    }

    /// Load coordinator configuration from file
    pub fn load_config(storage: &Storage) -> Result<CoordinatorConfig> {
        let config_path = storage.coordinator_config_path();

        if !config_path.exists() {
            // Return default config if file doesn't exist
            return Ok(CoordinatorConfig::default());
        }

        let content = fs::read_to_string(&config_path)?;
        let config: CoordinatorConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    /// Save coordinator configuration to file
    pub fn save_config(storage: &Storage, config: &CoordinatorConfig) -> Result<()> {
        let config_path = storage.coordinator_config_path();
        let content = serde_json::to_string_pretty(config)?;
        fs::write(config_path, content)?;
        Ok(())
    }

    /// Start the coordinator daemon
    pub fn start(&mut self) -> Result<()> {
        println!("Starting coordinator daemon...");
        println!(
            "Agent pool: {} agents configured",
            self.config.agent_pool.len()
        );
        println!("Poll interval: {} seconds", self.config.poll_interval_secs);

        let pid_path = self.storage.coordinator_pid_path();

        // Check if already running
        if pid_path.exists() {
            let pid = fs::read_to_string(&pid_path)?;
            bail!("Coordinator already running (PID: {})", pid.trim());
        }

        // Write PID file
        let pid = std::process::id();
        fs::write(&pid_path, pid.to_string())?;

        println!("Coordinator started (PID: {})", pid);
        println!("Press Ctrl+C to stop");

        // Main coordination loop
        loop {
            if let Err(e) = self.coordination_cycle() {
                eprintln!("Error in coordination cycle: {}", e);
            }

            thread::sleep(Duration::from_secs(self.config.poll_interval_secs));
        }
    }

    /// Run one cycle of the coordination loop
    fn coordination_cycle(&mut self) -> Result<()> {
        // Find ready-to-work issues
        let ready_issues = self.find_ready_issues()?;

        if ready_issues.is_empty() {
            return Ok(());
        }

        println!("Found {} ready issues", ready_issues.len());

        // Dispatch to available agents
        for issue in ready_issues {
            if let Err(e) = self.try_dispatch_issue(&issue) {
                eprintln!("Failed to dispatch issue {}: {}", issue.id, e);
            }
        }

        Ok(())
    }

    /// Find issues that are ready to be worked on
    fn find_ready_issues(&self) -> Result<Vec<Issue>> {
        let all_issues = self.storage.load_all_issues()?;

        let mut ready: Vec<Issue> = all_issues
            .into_iter()
            .filter(|issue| issue.state == State::Ready && issue.assignee.is_none())
            .collect();

        // Sort by priority
        ready.sort_by(|a, b| {
            let a_idx = self.priority_index(a.priority);
            let b_idx = self.priority_index(b.priority);
            a_idx.cmp(&b_idx)
        });

        Ok(ready)
    }

    fn priority_index(&self, priority: Priority) -> usize {
        let priority_str = match priority {
            Priority::Critical => "critical",
            Priority::High => "high",
            Priority::Normal => "normal",
            Priority::Low => "low",
        };

        self.config
            .dispatch_rules
            .priority_order
            .iter()
            .position(|p| p == priority_str)
            .unwrap_or(999)
    }

    /// Try to dispatch an issue to an available agent
    fn try_dispatch_issue(&mut self, issue: &Issue) -> Result<()> {
        // Find an available agent (clone to avoid borrow issues)
        let agent_pool = self.config.agent_pool.clone();

        for agent_config in agent_pool {
            let running = self
                .running_agents
                .get(&agent_config.id)
                .map(|v| v.len())
                .unwrap_or(0);

            if running < agent_config.max_concurrent {
                return self.dispatch_to_agent(issue, &agent_config);
            }
        }

        // No available agents
        Ok(())
    }

    /// Dispatch an issue to a specific agent
    fn dispatch_to_agent(&mut self, issue: &Issue, agent: &AgentConfig) -> Result<()> {
        let assignee = format!("{}:{}", agent.agent_type, agent.id);

        println!("Dispatching issue {} to agent {}", issue.id, assignee);

        // Claim the issue
        let mut updated_issue = issue.clone();
        updated_issue.assignee = Some(assignee.clone());
        updated_issue.state = State::InProgress;
        self.storage.save_issue(&updated_issue)?;

        // Log event
        let event = Event::new_issue_claimed(updated_issue.id.clone(), assignee.clone());
        self.storage.append_event(&event)?;

        // Execute agent command
        let cmd_parts: Vec<&str> = agent.command.split_whitespace().collect();
        if cmd_parts.is_empty() {
            bail!("Empty command for agent {}", agent.id);
        }

        let child = Command::new(cmd_parts[0])
            .args(&cmd_parts[1..])
            .arg(&issue.id)
            .spawn()?;

        // Track running agent
        self.running_agents
            .entry(agent.id.clone())
            .or_default()
            .push(child);

        Ok(())
    }

    /// Stop the coordinator daemon
    pub fn stop(storage: &Storage) -> Result<()> {
        let pid_path = storage.coordinator_pid_path();

        if !pid_path.exists() {
            println!("Coordinator is not running");
            return Ok(());
        }

        let pid_str = fs::read_to_string(&pid_path)?;
        let pid: u32 = pid_str.trim().parse()?;

        println!("Stopping coordinator (PID: {})...", pid);

        #[cfg(unix)]
        {
            use std::process::Command as SysCommand;
            SysCommand::new("kill").arg(pid.to_string()).output()?;
        }

        #[cfg(windows)]
        {
            use std::process::Command as SysCommand;
            SysCommand::new("taskkill")
                .args(&["/PID", &pid.to_string(), "/F"])
                .output()?;
        }

        fs::remove_file(&pid_path)?;
        println!("Coordinator stopped");

        Ok(())
    }

    /// Get coordinator status
    pub fn status(storage: &Storage) -> Result<()> {
        let pid_path = storage.coordinator_pid_path();

        if !pid_path.exists() {
            println!("Status: Not running");
            return Ok(());
        }

        let pid = fs::read_to_string(&pid_path)?;
        println!("Status: Running (PID: {})", pid.trim());

        // Load config
        let config = Self::load_config(storage)?;
        println!("Agent pool: {} agents", config.agent_pool.len());
        for agent in &config.agent_pool {
            println!(
                "  - {} (type: {}, max_concurrent: {})",
                agent.id, agent.agent_type, agent.max_concurrent
            );
        }

        // Show issue statistics
        let all_issues = storage.load_all_issues()?;
        let ready_count = all_issues
            .iter()
            .filter(|i| i.state == State::Ready && i.assignee.is_none())
            .count();
        let in_progress_count = all_issues
            .iter()
            .filter(|i| i.state == State::InProgress)
            .count();

        println!("\nWork queue:");
        println!("  Ready: {}", ready_count);
        println!("  In Progress: {}", in_progress_count);

        Ok(())
    }

    /// List active agents and their current assignments
    pub fn list_agents(storage: &Storage) -> Result<()> {
        let all_issues = storage.load_all_issues()?;
        let mut agent_map: HashMap<String, Vec<&Issue>> = HashMap::new();

        // Group issues by assignee
        for issue in &all_issues {
            if let Some(ref assignee) = issue.assignee {
                if issue.state == State::InProgress {
                    agent_map.entry(assignee.clone()).or_default().push(issue);
                }
            }
        }

        if agent_map.is_empty() {
            println!("No active agents");
            return Ok(());
        }

        println!("Active agents:");
        for (agent_id, issues) in agent_map {
            println!("\n{}", agent_id);
            for issue in issues {
                println!("  - {} | {}", issue.id, issue.title);
            }
        }

        Ok(())
    }
}
