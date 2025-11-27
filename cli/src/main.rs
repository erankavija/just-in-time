//! Just-In-Time Issue Tracker
//!
//! A repository-local CLI issue tracker with dependency graph enforcement and quality gating.
//! Designed for deterministic, machine-friendly outputs and process automation.
//!
//! # Features
//!
//! - Dependency graph modeling with cycle detection
//! - Quality gate enforcement before state transitions
//! - Event logging for full audit trail
//! - Priority-based issue management
//! - Agent coordination support

mod cli;
mod commands;
mod coordinator;
mod domain;
mod graph;
mod storage;

use anyhow::Result;
use clap::Parser;
use cli::{
    Cli, Commands, CoordinatorCommands, DepCommands, EventCommands, GateCommands, GraphCommands,
    IssueCommands, RegistryCommands,
};
use commands::{parse_priority, parse_state, CommandExecutor};
use coordinator::{AgentConfig, Coordinator, CoordinatorConfig, DispatchRules};
use std::env;
use storage::Storage;

fn main() -> Result<()> {
    let cli = Cli::parse();

    let current_dir = env::current_dir()?;
    let storage = Storage::new(&current_dir);
    let executor = CommandExecutor::new(storage.clone());

    match cli.command {
        Commands::Init => {
            executor.init()?;
        }
        Commands::Issue(issue_cmd) => match issue_cmd {
            IssueCommands::Create {
                title,
                desc,
                priority,
                gate,
            } => {
                let prio = parse_priority(&priority)?;
                let id = executor.create_issue(title, desc, prio, gate)?;
                println!("Created issue: {}", id);
            }
            IssueCommands::List {
                state,
                assignee,
                priority,
            } => {
                let state_filter = state.map(|s| parse_state(&s)).transpose()?;
                let priority_filter = priority.map(|p| parse_priority(&p)).transpose()?;
                let issues = executor.list_issues(state_filter, assignee, priority_filter)?;

                for issue in issues {
                    println!(
                        "{} | {} | {:?} | {:?}",
                        issue.id, issue.title, issue.state, issue.priority
                    );
                }
            }
            IssueCommands::Search {
                query,
                state,
                assignee,
                priority,
            } => {
                let state_filter = state.map(|s| parse_state(&s)).transpose()?;
                let priority_filter = priority.map(|p| parse_priority(&p)).transpose()?;
                let issues = executor.search_issues_with_filters(
                    &query,
                    priority_filter,
                    state_filter,
                    assignee,
                )?;

                println!("Found {} issue(s):", issues.len());
                for issue in issues {
                    println!(
                        "{} | {} | {:?} | {:?}",
                        issue.id, issue.title, issue.state, issue.priority
                    );
                }
            }
            IssueCommands::Show { id } => {
                let issue = executor.show_issue(&id)?;
                println!("ID: {}", issue.id);
                println!("Title: {}", issue.title);
                println!("Description: {}", issue.description);
                println!("State: {:?}", issue.state);
                println!("Priority: {:?}", issue.priority);
                println!("Assignee: {:?}", issue.assignee);
                println!("Dependencies: {:?}", issue.dependencies);
                println!("Gates Required: {:?}", issue.gates_required);
                println!("Gates Status: {:?}", issue.gates_status);
            }
            IssueCommands::Update {
                id,
                title,
                desc,
                priority,
                state,
            } => {
                let prio = priority.map(|p| parse_priority(&p)).transpose()?;
                let st = state.map(|s| parse_state(&s)).transpose()?;
                executor.update_issue(&id, title, desc, prio, st)?;
                println!("Updated issue: {}", id);
            }
            IssueCommands::Delete { id } => {
                executor.delete_issue(&id)?;
                println!("Deleted issue: {}", id);
            }
            IssueCommands::Assign { id, to } => {
                executor.assign_issue(&id, to)?;
                println!("Assigned issue: {}", id);
            }
            IssueCommands::Claim { id, to } => {
                executor.claim_issue(&id, to)?;
                println!("Claimed issue: {}", id);
            }
            IssueCommands::Unassign { id } => {
                executor.unassign_issue(&id)?;
                println!("Unassigned issue: {}", id);
            }
            IssueCommands::ClaimNext { to, filter } => {
                let id = executor.claim_next(to, filter)?;
                println!("Claimed issue: {}", id);
            }
        },
        Commands::Dep(dep_cmd) => match dep_cmd {
            DepCommands::Add { id, on } => {
                executor.add_dependency(&id, &on)?;
                println!("Added dependency: {} depends on {}", id, on);
            }
            DepCommands::Rm { id, on } => {
                executor.remove_dependency(&id, &on)?;
                println!("Removed dependency: {} no longer depends on {}", id, on);
            }
        },
        Commands::Gate(gate_cmd) => match gate_cmd {
            GateCommands::Add { id, gate_key } => {
                executor.add_gate(&id, gate_key.clone())?;
                println!("Added gate '{}' to issue {}", gate_key, id);
            }
            GateCommands::Pass { id, gate_key, by } => {
                executor.pass_gate(&id, gate_key.clone(), by)?;
                println!("Passed gate '{}' for issue {}", gate_key, id);
            }
            GateCommands::Fail { id, gate_key, by } => {
                executor.fail_gate(&id, gate_key.clone(), by)?;
                println!("Failed gate '{}' for issue {}", gate_key, id);
            }
        },
        Commands::Graph(graph_cmd) => match graph_cmd {
            GraphCommands::Show { id } => {
                let issues = executor.show_graph(&id)?;
                println!("Dependency tree for {}:", id);
                for issue in issues {
                    println!("  {} | {}", issue.id, issue.title);
                }
            }
            GraphCommands::Downstream { id } => {
                let issues = executor.show_downstream(&id)?;
                println!("Downstream dependents of {}:", id);
                for issue in issues {
                    println!("  {} | {}", issue.id, issue.title);
                }
            }
            GraphCommands::Roots => {
                let issues = executor.show_roots()?;
                println!("Root issues (no dependencies):");
                for issue in issues {
                    println!("  {} | {}", issue.id, issue.title);
                }
            }
            GraphCommands::Export { format, output } => {
                let graph_output = executor.export_graph(&format)?;

                if let Some(path) = output {
                    std::fs::write(&path, graph_output)?;
                    println!("Graph exported to: {}", path);
                } else {
                    println!("{}", graph_output);
                }
            }
        },
        Commands::Registry(registry_cmd) => match registry_cmd {
            RegistryCommands::List => {
                let gates = executor.list_gates()?;
                for gate in gates {
                    println!("{} | {} | auto:{}", gate.key, gate.title, gate.auto);
                }
            }
            RegistryCommands::Add {
                key,
                title,
                desc,
                auto,
                example,
            } => {
                executor.add_gate_definition(key.clone(), title, desc, auto, example)?;
                println!("Added gate definition: {}", key);
            }
            RegistryCommands::Remove { key } => {
                executor.remove_gate_definition(&key)?;
                println!("Removed gate definition: {}", key);
            }
            RegistryCommands::Show { key } => {
                let gate = executor.show_gate_definition(&key)?;
                println!("Key: {}", gate.key);
                println!("Title: {}", gate.title);
                println!("Description: {}", gate.description);
                println!("Auto: {}", gate.auto);
                println!("Example Integration: {:?}", gate.example_integration);
            }
        },
        Commands::Events(event_cmd) => match event_cmd {
            EventCommands::Tail { n } => {
                let events = executor.tail_events(n)?;
                for event in events {
                    println!("{}", serde_json::to_string(&event)?);
                }
            }
            EventCommands::Query {
                event_type,
                issue_id,
                limit,
            } => {
                let events = executor.query_events(event_type, issue_id, limit)?;
                for event in events {
                    println!("{}", serde_json::to_string(&event)?);
                }
            }
        },
        Commands::Coordinator(coord_cmd) => match coord_cmd {
            CoordinatorCommands::Start { config } => {
                let coord_config = if let Some(config_path) = config {
                    let content = std::fs::read_to_string(&config_path)?;
                    serde_json::from_str(&content)?
                } else {
                    Coordinator::load_config(&storage)?
                };

                let mut coordinator = Coordinator::new(storage, coord_config);
                coordinator.start()?;
            }
            CoordinatorCommands::Stop => {
                Coordinator::stop(&storage)?;
            }
            CoordinatorCommands::Status => {
                Coordinator::status(&storage)?;
            }
            CoordinatorCommands::Agents => {
                Coordinator::list_agents(&storage)?;
            }
            CoordinatorCommands::InitConfig => {
                let config = CoordinatorConfig {
                    agent_pool: vec![
                        AgentConfig {
                            id: "copilot-1".to_string(),
                            agent_type: "copilot".to_string(),
                            command: "echo Processing issue".to_string(),
                            max_concurrent: 2,
                        },
                        AgentConfig {
                            id: "copilot-2".to_string(),
                            agent_type: "copilot".to_string(),
                            command: "echo Processing issue".to_string(),
                            max_concurrent: 2,
                        },
                    ],
                    dispatch_rules: DispatchRules::default(),
                    poll_interval_secs: 5,
                };

                Coordinator::save_config(&storage, &config)?;
                println!("Created example coordinator config at data/coordinator.json");
                println!("Edit the config to customize agent commands and settings");
            }
        },
        Commands::Status => {
            executor.status()?;
        }
        Commands::Validate => {
            executor.validate()?;
        }
    }

    Ok(())
}
