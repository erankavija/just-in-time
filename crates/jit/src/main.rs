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
mod domain;
mod graph;
mod output;
mod storage;
mod visualization;

use anyhow::Result;
use clap::Parser;
use cli::{
    Cli, Commands, DepCommands, EventCommands, GateCommands, GraphCommands, IssueCommands,
    RegistryCommands,
};
use commands::{parse_priority, parse_state, CommandExecutor};
use std::env;
use storage::{IssueStore, JsonFileStorage};

fn main() -> Result<()> {
    let cli = Cli::parse();

    let current_dir = env::current_dir()?;
    let storage = JsonFileStorage::new(&current_dir);
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
                json,
            } => {
                let prio = parse_priority(&priority)?;
                let id = executor.create_issue(title, desc, prio, gate)?;

                if json {
                    let issue = storage.load_issue(&id)?;
                    println!("{}", serde_json::to_string_pretty(&issue)?);
                } else {
                    println!("Created issue: {}", id);
                }
            }
            IssueCommands::List {
                state,
                assignee,
                priority,
                json,
            } => {
                let state_filter = state.map(|s| parse_state(&s)).transpose()?;
                let priority_filter = priority.map(|p| parse_priority(&p)).transpose()?;
                let issues = executor.list_issues(state_filter, assignee, priority_filter)?;

                if json {
                    use output::JsonOutput;
                    use serde_json::json;
                    
                    // Count issues by state
                    let mut state_counts = std::collections::HashMap::new();
                    for issue in &issues {
                        *state_counts.entry(issue.state).or_insert(0) += 1;
                    }
                    
                    let output = JsonOutput::success(json!({
                        "issues": issues,
                        "summary": {
                            "total": issues.len(),
                            "by_state": state_counts,
                        }
                    }));
                    println!("{}", output.to_json_string()?);
                } else {
                    for issue in issues {
                        println!(
                            "{} | {} | {:?} | {:?}",
                            issue.id, issue.title, issue.state, issue.priority
                        );
                    }
                }
            }
            IssueCommands::Search {
                query,
                state,
                assignee,
                priority,
                json,
            } => {
                let state_filter = state.map(|s| parse_state(&s)).transpose()?;
                let priority_filter = priority.map(|p| parse_priority(&p)).transpose()?;
                let issues = executor.search_issues_with_filters(
                    &query,
                    priority_filter,
                    state_filter,
                    assignee,
                )?;

                if json {
                    use output::JsonOutput;
                    use serde_json::json;
                    
                    let output = JsonOutput::success(json!({
                        "query": query,
                        "issues": issues,
                        "count": issues.len(),
                    }));
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Found {} issue(s):", issues.len());
                    for issue in issues {
                        println!(
                            "{} | {} | {:?} | {:?}",
                            issue.id, issue.title, issue.state, issue.priority
                        );
                    }
                }
            }
            IssueCommands::Show { id, json } => {
                let issue = executor.show_issue(&id)?;
                if json {
                    use output::JsonOutput;
                    
                    let output = JsonOutput::success(&issue);
                    println!("{}", output.to_json_string()?);
                } else {
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
            }
            IssueCommands::Update {
                id,
                title,
                desc,
                priority,
                state,
                json,
            } => {
                let prio = priority.map(|p| parse_priority(&p)).transpose()?;
                let st = state.map(|s| parse_state(&s)).transpose()?;
                executor.update_issue(&id, title, desc, prio, st)?;

                if json {
                    let issue = storage.load_issue(&id)?;
                    println!("{}", serde_json::to_string_pretty(&issue)?);
                } else {
                    println!("Updated issue: {}", id);
                }
            }
            IssueCommands::Delete { id, json } => {
                executor.delete_issue(&id)?;

                if json {
                    let result = serde_json::json!({
                        "id": id,
                        "deleted": true
                    });
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!("Deleted issue: {}", id);
                }
            }
            IssueCommands::Assign { id, assignee, json } => {
                executor.assign_issue(&id, assignee)?;

                if json {
                    let issue = storage.load_issue(&id)?;
                    println!("{}", serde_json::to_string_pretty(&issue)?);
                } else {
                    println!("Assigned issue: {}", id);
                }
            }
            IssueCommands::Claim { id, assignee, json } => {
                executor.claim_issue(&id, assignee)?;

                if json {
                    let issue = storage.load_issue(&id)?;
                    println!("{}", serde_json::to_string_pretty(&issue)?);
                } else {
                    println!("Claimed issue: {}", id);
                }
            }
            IssueCommands::Unassign { id, json } => {
                executor.unassign_issue(&id)?;

                if json {
                    let issue = storage.load_issue(&id)?;
                    println!("{}", serde_json::to_string_pretty(&issue)?);
                } else {
                    println!("Unassigned issue: {}", id);
                }
            }
            IssueCommands::Release { id, reason, json } => {
                executor.release_issue(&id, &reason)?;

                if json {
                    let issue = storage.load_issue(&id)?;
                    println!("{}", serde_json::to_string_pretty(&issue)?);
                } else {
                    println!("Released issue: {} (reason: {})", id, reason);
                }
            }
            IssueCommands::ClaimNext { assignee, filter } => {
                let id = executor.claim_next(assignee, filter)?;
                println!("Claimed issue: {}", id);
            }
        },
        Commands::Dep(dep_cmd) => match dep_cmd {
            DepCommands::Add { from_id, to_id } => {
                executor.add_dependency(&from_id, &to_id)?;
                println!("Added dependency: {} depends on {}", from_id, to_id);
            }
            DepCommands::Rm { from_id, to_id } => {
                executor.remove_dependency(&from_id, &to_id)?;
                println!(
                    "Removed dependency: {} no longer depends on {}",
                    from_id, to_id
                );
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
                if let Some(issue_id) = id {
                    let issues = executor.show_graph(&issue_id)?;
                    println!("Dependency tree for {}:", issue_id);
                    for issue in issues {
                        println!("  {} | {}", issue.id, issue.title);
                    }
                } else {
                    // Show all dependencies as a graph
                    let all_issues = executor.list_issues(None, None, None)?;
                    println!("All dependencies:");
                    for issue in all_issues {
                        if !issue.dependencies.is_empty() {
                            println!("  {} depends on: {:?}", issue.id, issue.dependencies);
                        }
                    }
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
        Commands::Query(query_cmd) => match query_cmd {
            cli::QueryCommands::Ready { json } => {
                let issues = executor.query_ready()?;
                if json {
                    let response = serde_json::json!({
                        "issues": issues,
                        "count": issues.len(),
                        "timestamp": chrono::Utc::now(),
                    });
                    println!("{}", serde_json::to_string_pretty(&response)?);
                } else {
                    println!("Ready issues (unassigned, unblocked):");
                    for issue in &issues {
                        println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                    }
                    println!("\nTotal: {}", issues.len());
                }
            }
            cli::QueryCommands::Blocked { json } => {
                let blocked = executor.query_blocked()?;
                if json {
                    let issues_with_reasons: Vec<serde_json::Value> = blocked
                        .iter()
                        .map(|(issue, reasons)| {
                            serde_json::json!({
                                "id": issue.id,
                                "title": issue.title,
                                "state": issue.state,
                                "priority": issue.priority,
                                "blocked_reasons": reasons.iter().map(|r| {
                                    let parts: Vec<&str> = r.splitn(2, ':').collect();
                                    serde_json::json!({
                                        "type": parts[0],
                                        "detail": parts.get(1).unwrap_or(&""),
                                    })
                                }).collect::<Vec<_>>()
                            })
                        })
                        .collect();
                    let response = serde_json::json!({
                        "issues": issues_with_reasons,
                        "count": blocked.len(),
                        "timestamp": chrono::Utc::now(),
                    });
                    println!("{}", serde_json::to_string_pretty(&response)?);
                } else {
                    println!("Blocked issues:");
                    for (issue, reasons) in &blocked {
                        println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                        for reason in reasons {
                            println!("    - {}", reason);
                        }
                    }
                    println!("\nTotal: {}", blocked.len());
                }
            }
            cli::QueryCommands::Assignee { assignee, json } => {
                let issues = executor.query_by_assignee(&assignee)?;
                if json {
                    let response = serde_json::json!({
                        "issues": issues,
                        "count": issues.len(),
                        "timestamp": chrono::Utc::now(),
                    });
                    println!("{}", serde_json::to_string_pretty(&response)?);
                } else {
                    println!("Issues assigned to {}:", assignee);
                    for issue in &issues {
                        println!(
                            "  {} | {} | {:?} | {:?}",
                            issue.id, issue.title, issue.state, issue.priority
                        );
                    }
                    println!("\nTotal: {}", issues.len());
                }
            }
            cli::QueryCommands::State { state, json } => {
                let parsed_state = parse_state(&state)?;
                let issues = executor.query_by_state(parsed_state)?;
                if json {
                    let response = serde_json::json!({
                        "issues": issues,
                        "count": issues.len(),
                        "timestamp": chrono::Utc::now(),
                    });
                    println!("{}", serde_json::to_string_pretty(&response)?);
                } else {
                    println!("Issues with state '{}':", state);
                    for issue in &issues {
                        println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                    }
                    println!("\nTotal: {}", issues.len());
                }
            }
            cli::QueryCommands::Priority { priority, json } => {
                let parsed_priority = parse_priority(&priority)?;
                let issues = executor.query_by_priority(parsed_priority)?;
                if json {
                    let response = serde_json::json!({
                        "issues": issues,
                        "count": issues.len(),
                        "timestamp": chrono::Utc::now(),
                    });
                    println!("{}", serde_json::to_string_pretty(&response)?);
                } else {
                    println!("Issues with priority '{}':", priority);
                    for issue in &issues {
                        println!("  {} | {} | {:?}", issue.id, issue.title, issue.state);
                    }
                    println!("\nTotal: {}", issues.len());
                }
            }
        },
        Commands::Status { json } => {
            if json {
                use output::JsonOutput;
                
                let summary = executor.get_status()?;
                let output = JsonOutput::success(&summary);
                println!("{}", output.to_json_string()?);
            } else {
                executor.status()?;
            }
        }
        Commands::Validate { json } => {
            if json {
                use output::JsonOutput;
                use serde_json::json;
                
                executor.validate_silent()?;
                let output = JsonOutput::success(json!({
                    "valid": true,
                    "message": "Repository validation passed"
                }));
                println!("{}", output.to_json_string()?);
            } else {
                executor.validate()?;
            }
        }
    }

    Ok(())
}
