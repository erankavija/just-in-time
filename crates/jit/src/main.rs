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

// Binary-specific module (not in library)
mod output_macros;

use anyhow::{anyhow, Result};
use clap::Parser;
use jit::cli::{
    Cli, Commands, DepCommands, DocCommands, EventCommands, GateCommands, GraphCommands,
    IssueCommands, RegistryCommands,
};
use jit::commands::{parse_priority, parse_state, CommandExecutor};
use jit::output::ExitCode;
use jit::storage::{IssueStore, JsonFileStorage};
use std::env;

/// Helper to determine exit code from error message
fn error_to_exit_code(error: &anyhow::Error) -> ExitCode {
    let error_msg = error.to_string().to_lowercase();

    // Check root cause for IO errors
    if let Some(io_error) = error.downcast_ref::<std::io::Error>() {
        return match io_error.kind() {
            std::io::ErrorKind::NotFound => ExitCode::NotFound,
            std::io::ErrorKind::PermissionDenied => ExitCode::PermissionDenied,
            _ => ExitCode::ExternalError,
        };
    }

    // Check error message patterns
    if error_msg.contains("not found") || error_msg.contains("no such file") {
        ExitCode::NotFound
    } else if error_msg.contains("cycle") || error_msg.contains("invalid dependency") {
        ExitCode::ValidationFailed
    } else if error_msg.contains("already exists") {
        ExitCode::AlreadyExists
    } else if error_msg.contains("invalid") && !error_msg.contains("invalid dependency") {
        ExitCode::InvalidArgument
    } else if error_msg.contains("data")
        && (error_msg.contains("failed to read") || error_msg.contains("io error"))
    {
        // File system errors related to data directory
        ExitCode::ExternalError
    } else {
        ExitCode::GenericError
    }
}

fn main() {
    let exit_code = match run() {
        Ok(()) => ExitCode::Success,
        Err(e) => {
            eprintln!("Error: {}", e);
            error_to_exit_code(&e)
        }
    };

    if exit_code != ExitCode::Success {
        std::process::exit(exit_code.code());
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    // Handle --schema flag first
    if cli.schema {
        use jit::CommandSchema;
        let schema = CommandSchema::generate();
        let json = serde_json::to_string_pretty(&schema)?;
        println!("{}", json);
        return Ok(());
    }

    if cli.schema_auto {
        // Deprecated: kept for backwards compatibility, same as --schema
        use jit::CommandSchema;
        let schema = CommandSchema::generate();
        let json = serde_json::to_string_pretty(&schema)?;
        println!("{}", json);
        return Ok(());
    }

    // Ensure command is provided
    let command = cli
        .command
        .ok_or_else(|| anyhow::anyhow!("No command provided. Use --help for usage."))?;

    let current_dir = env::current_dir()?;

    // Determine jit data directory: JIT_DATA_DIR env var or default to .jit/
    let jit_dir = if let Ok(custom_dir) = env::var("JIT_DATA_DIR") {
        current_dir.join(custom_dir)
    } else {
        current_dir.join(".jit")
    };

    let storage = JsonFileStorage::new(&jit_dir);
    let mut executor = CommandExecutor::new(storage.clone());

    match &command {
        Commands::Init { hierarchy_template } => {
            executor.init()?;

            // If a template is specified, update the labels config with the hierarchy
            if let Some(template_name) = hierarchy_template {
                let template = jit::hierarchy_templates::HierarchyTemplate::get(template_name)
                    .ok_or_else(|| anyhow!("Unknown hierarchy template: {}", template_name))?;

                let mut namespaces = executor.storage().load_label_namespaces()?;
                namespaces.type_hierarchy = Some(template.hierarchy);
                namespaces.label_associations = Some(template.label_associations);
                namespaces.schema_version = 2;

                // Dynamically create/update membership namespaces
                namespaces.sync_membership_namespaces();

                executor.storage().save_label_namespaces(&namespaces)?;

                println!("Initialized with '{}' hierarchy template", template_name);
            }
        }
        _ => {
            // Validate repository exists for all commands except init
            storage.validate()?;
        }
    }

    match command {
        Commands::Init { .. } => {
            // Already handled above
        }
        Commands::Issue(issue_cmd) => {
            match issue_cmd {
                IssueCommands::Create {
                    title,
                    desc,
                    priority,
                    gate,
                    label,
                    force,
                    orphan,
                    json,
                } => {
                    let prio = parse_priority(&priority)?;
                    let id = executor.create_issue(title, desc, prio, gate, label)?;

                    if json {
                        let issue = storage.load_issue(&id)?;
                        println!("{}", serde_json::to_string_pretty(&issue)?);
                    } else {
                        println!("Created issue: {}", id);

                        // Check for warnings unless --force is set
                        if !force {
                            use jit::type_hierarchy::ValidationWarning;

                            let warnings = executor.check_warnings(&id)?;

                            // Filter orphan warnings if --orphan flag is set
                            let warnings_to_display: Vec<_> = if orphan {
                                warnings
                                    .into_iter()
                                    .filter(|w| {
                                        !matches!(w, ValidationWarning::OrphanedLeaf { .. })
                                    })
                                    .collect()
                            } else {
                                warnings
                            };

                            // Display warnings
                            for warning in warnings_to_display {
                                match warning {
                                    ValidationWarning::MissingStrategicLabel {
                                        type_name,
                                        expected_namespace,
                                        ..
                                    } => {
                                        eprintln!("\n⚠ Warning: Strategic consistency issue");
                                        eprintln!("  Issue {} (type:{}) should have a {}:* label for identification.", 
                                             id, type_name, expected_namespace);
                                        eprintln!(
                                            "  Suggested: jit issue update {} --label \"{}:value\"",
                                            id, expected_namespace
                                        );
                                    }
                                    ValidationWarning::OrphanedLeaf { type_name, .. } => {
                                        eprintln!("\n⚠ Warning: Orphaned leaf issue");
                                        eprintln!("  {} {} has no parent association (epic or milestone).", 
                                             type_name.to_uppercase(), id);
                                        eprintln!("  Consider adding: --label \"epic:value\" or --label \"milestone:value\"");
                                        eprintln!("  Or use --orphan flag to acknowledge intentional orphan.");
                                    }
                                }
                            }
                        }
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
                        use jit::output::JsonOutput;
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
                        use jit::output::JsonOutput;
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
                IssueCommands::Show { id, json } => match executor.show_issue(&id) {
                    Ok(issue) => {
                        output_data!(json, issue, {
                            println!("ID: {}", issue.id);
                            println!("Title: {}", issue.title);
                            println!("Description: {}", issue.description);
                            println!("State: {:?}", issue.state);
                            println!("Priority: {:?}", issue.priority);
                            println!("Assignee: {:?}", issue.assignee);
                            println!("Dependencies: {:?}", issue.dependencies);
                            println!("Gates Required: {:?}", issue.gates_required);
                            println!("Gates Status: {:?}", issue.gates_status);
                            if !issue.documents.is_empty() {
                                println!("Documents:");
                                for doc in &issue.documents {
                                    print!("  - {}", doc.path);
                                    if let Some(ref label) = doc.label {
                                        print!(" ({})", label);
                                    }
                                    if let Some(ref commit) = doc.commit {
                                        print!(" [{}]", &commit[..7.min(commit.len())]);
                                    } else {
                                        print!(" [HEAD]");
                                    }
                                    println!();
                                }
                            }
                        });
                    }
                    Err(e) => {
                        handle_json_error!(json, e, jit::output::JsonError::issue_not_found(&id));
                    }
                },
                IssueCommands::Update {
                    id,
                    title,
                    desc,
                    priority,
                    state,
                    label,
                    remove_label,
                    json,
                } => {
                    let prio = priority.map(|p| parse_priority(&p)).transpose()?;
                    let st = state.map(|s| parse_state(&s)).transpose()?;
                    executor.update_issue(&id, title, desc, prio, st, label, remove_label)?;

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
                IssueCommands::Breakdown {
                    parent_id,
                    subtask_titles,
                    subtask_descs,
                    json,
                } => {
                    // Pad descriptions with empty strings if not enough provided
                    let mut descs = subtask_descs.clone();
                    while descs.len() < subtask_titles.len() {
                        descs.push(String::new());
                    }

                    let subtasks: Vec<(String, String)> = subtask_titles
                        .iter()
                        .zip(descs.iter())
                        .map(|(t, d)| (t.clone(), d.clone()))
                        .collect();

                    let subtask_ids = executor.breakdown_issue(&parent_id, subtasks)?;

                    if json {
                        use jit::output::JsonOutput;
                        let response = serde_json::json!({
                            "parent_id": parent_id,
                            "subtask_ids": subtask_ids,
                            "count": subtask_ids.len(),
                            "message": format!("Broke down {} into {} subtasks", parent_id, subtask_ids.len())
                        });
                        let output = JsonOutput::success(response);
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!(
                            "Broke down {} into {} subtasks:",
                            parent_id,
                            subtask_ids.len()
                        );
                        for (i, id) in subtask_ids.iter().enumerate() {
                            println!("  {}. {}", i + 1, id);
                        }
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
            }
        }
        Commands::Dep(dep_cmd) => match dep_cmd {
            DepCommands::Add {
                from_id,
                to_id,
                json,
            } => match executor.add_dependency(&from_id, &to_id) {
                Ok(result) => {
                    use jit::commands::DependencyAddResult;
                    if json {
                        use jit::output::JsonOutput;
                        let (status, message) = match result {
                            DependencyAddResult::Added => {
                                ("added", format!("{} now depends on {}", from_id, to_id))
                            }
                            DependencyAddResult::Skipped { reason } => {
                                ("skipped", format!("Dependency skipped: {}", reason))
                            }
                            DependencyAddResult::AlreadyExists => (
                                "exists",
                                format!("{} already depends on {}", from_id, to_id),
                            ),
                        };
                        let response = serde_json::json!({
                            "from_id": from_id,
                            "to_id": to_id,
                            "status": status,
                            "message": message
                        });
                        let output = JsonOutput::success(response);
                        println!("{}", output.to_json_string()?);
                    } else {
                        match result {
                            DependencyAddResult::Added => {
                                println!("Added dependency: {} depends on {}", from_id, to_id);
                            }
                            DependencyAddResult::Skipped { reason } => {
                                println!("Skipped: dependency not added ({})", reason);
                            }
                            DependencyAddResult::AlreadyExists => {
                                println!(
                                    "Dependency already exists: {} depends on {}",
                                    from_id, to_id
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    if json {
                        use jit::output::JsonError;
                        let error_str = e.to_string();
                        let json_error = if error_str.contains("cycle") {
                            JsonError::cycle_detected(&from_id, &to_id)
                        } else if error_str.contains("not found") {
                            if error_str.contains(&from_id) {
                                JsonError::issue_not_found(&from_id)
                            } else {
                                JsonError::issue_not_found(&to_id)
                            }
                        } else {
                            JsonError::new("DEPENDENCY_ERROR", error_str)
                        };
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(json_error.exit_code().code());
                    } else {
                        return Err(e);
                    }
                }
            },
            DepCommands::Rm {
                from_id,
                to_id,
                json,
            } => match executor.remove_dependency(&from_id, &to_id) {
                Ok(_) => {
                    if json {
                        use jit::output::JsonOutput;
                        let response = serde_json::json!({
                            "from_id": from_id,
                            "to_id": to_id,
                            "message": format!("{} no longer depends on {}", from_id, to_id)
                        });
                        let output = JsonOutput::success(response);
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!(
                            "Removed dependency: {} no longer depends on {}",
                            from_id, to_id
                        );
                    }
                }
                Err(e) => {
                    if json {
                        use jit::output::JsonError;
                        let error_str = e.to_string();
                        let json_error = if error_str.contains("not found") {
                            if error_str.contains(&from_id) {
                                JsonError::issue_not_found(&from_id)
                            } else {
                                JsonError::issue_not_found(&to_id)
                            }
                        } else {
                            JsonError::new("DEPENDENCY_ERROR", error_str)
                        };
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(json_error.exit_code().code());
                    } else {
                        return Err(e);
                    }
                }
            },
        },
        Commands::Gate(gate_cmd) => match gate_cmd {
            GateCommands::Add { id, gate_key, json } => {
                match executor.add_gate(&id, gate_key.clone()) {
                    Ok(_) => {
                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "issue_id": id,
                                "gate_key": gate_key,
                                "message": format!("Added gate '{}' to issue {}", gate_key, id)
                            });
                            let output = JsonOutput::success(response);
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("Added gate '{}' to issue {}", gate_key, id);
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let error_str = e.to_string();
                            let json_error = if error_str.contains("Issue")
                                && error_str.contains("not found")
                            {
                                JsonError::issue_not_found(&id)
                            } else if error_str.contains("Gate") && error_str.contains("not found")
                            {
                                JsonError::gate_not_found(&gate_key)
                            } else {
                                JsonError::new("GATE_ERROR", error_str)
                            };
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::Pass {
                id,
                gate_key,
                by,
                json,
            } => match executor.pass_gate(&id, gate_key.clone(), by) {
                Ok(_) => {
                    if json {
                        use jit::output::JsonOutput;
                        let response = serde_json::json!({
                            "issue_id": id,
                            "gate_key": gate_key,
                            "status": "passed",
                            "message": format!("Passed gate '{}' for issue {}", gate_key, id)
                        });
                        let output = JsonOutput::success(response);
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!("Passed gate '{}' for issue {}", gate_key, id);
                    }
                }
                Err(e) => {
                    if json {
                        use jit::output::JsonError;
                        let json_error = JsonError::new("GATE_ERROR", e.to_string());
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(json_error.exit_code().code());
                    } else {
                        return Err(e);
                    }
                }
            },
            GateCommands::Fail {
                id,
                gate_key,
                by,
                json,
            } => match executor.fail_gate(&id, gate_key.clone(), by) {
                Ok(_) => {
                    if json {
                        use jit::output::JsonOutput;
                        let response = serde_json::json!({
                            "issue_id": id,
                            "gate_key": gate_key,
                            "status": "failed",
                            "message": format!("Failed gate '{}' for issue {}", gate_key, id)
                        });
                        let output = JsonOutput::success(response);
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!("Failed gate '{}' for issue {}", gate_key, id);
                    }
                }
                Err(e) => {
                    if json {
                        use jit::output::JsonError;
                        let json_error = JsonError::new("GATE_ERROR", e.to_string());
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(json_error.exit_code().code());
                    } else {
                        return Err(e);
                    }
                }
            },
        },
        Commands::Graph(graph_cmd) => match graph_cmd {
            GraphCommands::Show { id, json } => {
                if let Some(issue_id) = id {
                    let issues = executor.show_graph(&issue_id)?;
                    if json {
                        use jit::output::{GraphShowResponse, JsonOutput};

                        let response = GraphShowResponse {
                            issue_id: issue_id.clone(),
                            dependencies: issues.clone(),
                            count: issues.len(),
                        };
                        let output = JsonOutput::success(response);
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!("Dependency tree for {}:", issue_id);
                        for issue in issues {
                            println!("  {} | {}", issue.id, issue.title);
                        }
                    }
                } else {
                    // Show all dependencies as a graph
                    let all_issues = executor.list_issues(None, None, None)?;
                    if json {
                        use jit::output::{DependencyPair, GraphShowAllResponse, JsonOutput};

                        let mut deps = Vec::new();
                        for issue in &all_issues {
                            for dep_id in &issue.dependencies {
                                let dep_title = all_issues
                                    .iter()
                                    .find(|i| &i.id == dep_id)
                                    .map(|i| i.title.clone())
                                    .unwrap_or_else(|| "Unknown".to_string());
                                deps.push(DependencyPair {
                                    from_id: issue.id.clone(),
                                    from_title: issue.title.clone(),
                                    to_id: dep_id.clone(),
                                    to_title: dep_title,
                                });
                            }
                        }

                        let response = GraphShowAllResponse {
                            count: deps.len(),
                            dependencies: deps,
                        };
                        let output = JsonOutput::success(response);
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!("All dependencies:");
                        for issue in all_issues {
                            if !issue.dependencies.is_empty() {
                                println!("  {} depends on: {:?}", issue.id, issue.dependencies);
                            }
                        }
                    }
                }
            }
            GraphCommands::Downstream { id, json } => {
                let issues = executor.show_downstream(&id)?;
                if json {
                    use jit::output::{GraphDownstreamResponse, JsonOutput};

                    let response = GraphDownstreamResponse {
                        issue_id: id.clone(),
                        dependents: issues.clone(),
                        count: issues.len(),
                    };
                    let output = JsonOutput::success(response);
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Downstream dependents of {}:", id);
                    for issue in issues {
                        println!("  {} | {}", issue.id, issue.title);
                    }
                }
            }
            GraphCommands::Roots { json } => {
                let issues = executor.show_roots()?;
                if json {
                    use jit::output::{GraphRootsResponse, JsonOutput};

                    let response = GraphRootsResponse {
                        roots: issues.clone(),
                        count: issues.len(),
                    };
                    let output = JsonOutput::success(response);
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Root issues (no dependencies):");
                    for issue in issues {
                        println!("  {} | {}", issue.id, issue.title);
                    }
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
            RegistryCommands::List { json } => {
                let gates = executor.list_gates()?;
                if json {
                    use jit::output::{GateDefinition, JsonOutput, RegistryListResponse};

                    let gate_defs: Vec<GateDefinition> = gates
                        .iter()
                        .map(|g| GateDefinition {
                            key: g.key.clone(),
                            title: g.title.clone(),
                            description: g.description.clone(),
                            auto: g.auto,
                            example_integration: g.example_integration.clone(),
                        })
                        .collect();

                    let response = RegistryListResponse {
                        count: gate_defs.len(),
                        gates: gate_defs,
                    };
                    let output = JsonOutput::success(response);
                    println!("{}", output.to_json_string()?);
                } else {
                    for gate in gates {
                        println!("{} | {} | auto:{}", gate.key, gate.title, gate.auto);
                    }
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
            RegistryCommands::Show { key, json } => {
                let gate = executor.show_gate_definition(&key)?;
                if json {
                    use jit::output::{GateDefinition, JsonOutput};

                    let gate_def = GateDefinition {
                        key: gate.key.clone(),
                        title: gate.title.clone(),
                        description: gate.description.clone(),
                        auto: gate.auto,
                        example_integration: gate.example_integration.clone(),
                    };
                    let output = JsonOutput::success(gate_def);
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Key: {}", gate.key);
                    println!("Title: {}", gate.title);
                    println!("Description: {}", gate.description);
                    println!("Auto: {}", gate.auto);
                    println!("Example Integration: {:?}", gate.example_integration);
                }
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
        Commands::Doc(doc_cmd) => match doc_cmd {
            DocCommands::Add {
                id,
                path,
                commit,
                label,
                doc_type,
                json,
            } => {
                executor.add_document_reference(
                    &id,
                    &path,
                    commit.as_deref(),
                    label.as_deref(),
                    doc_type.as_deref(),
                    json,
                )?;
            }
            DocCommands::List { id, json } => {
                executor.list_document_references(&id, json)?;
            }
            DocCommands::Remove { id, path, json } => {
                executor.remove_document_reference(&id, &path, json)?;
            }
            DocCommands::Show { id, path, at } => {
                executor.show_document_content(&id, &path, at.as_deref())?;
            }
            DocCommands::History { id, path, json } => {
                executor.document_history(&id, &path, json)?;
            }
            DocCommands::Diff { id, path, from, to } => {
                executor.document_diff(&id, &path, &from, to.as_deref())?;
            }
        },
        Commands::Query(query_cmd) => match query_cmd {
            jit::cli::QueryCommands::Ready { json } => {
                let issues = executor.query_ready()?;
                if json {
                    use jit::output::{JsonOutput, ReadyQueryResponse};

                    let response = ReadyQueryResponse {
                        issues: issues.clone(),
                        count: issues.len(),
                    };
                    let output = JsonOutput::success(response);
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Ready issues (unassigned, unblocked):");
                    for issue in &issues {
                        println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                    }
                    println!("\nTotal: {}", issues.len());
                }
            }
            jit::cli::QueryCommands::Blocked { json } => {
                let blocked = executor.query_blocked()?;
                if json {
                    use jit::output::{
                        BlockedIssue, BlockedQueryResponse, BlockedReason, BlockedReasonType,
                        JsonOutput,
                    };

                    let blocked_issues: Vec<BlockedIssue> = blocked
                        .iter()
                        .map(|(issue, reasons)| {
                            let blocked_reasons = reasons
                                .iter()
                                .map(|r| {
                                    let parts: Vec<&str> = r.splitn(2, ':').collect();
                                    let reason_type = match parts[0] {
                                        "gate" => BlockedReasonType::Gate,
                                        _ => BlockedReasonType::Dependency,
                                    };
                                    BlockedReason {
                                        reason_type,
                                        detail: parts.get(1).unwrap_or(&"").to_string(),
                                    }
                                })
                                .collect();
                            BlockedIssue {
                                issue: (*issue).clone(),
                                blocked_reasons,
                            }
                        })
                        .collect();

                    let response = BlockedQueryResponse {
                        issues: blocked_issues,
                        count: blocked.len(),
                    };
                    let output = JsonOutput::success(response);
                    println!("{}", output.to_json_string()?);
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
            jit::cli::QueryCommands::Assignee { assignee, json } => {
                let issues = executor.query_by_assignee(&assignee)?;
                if json {
                    use jit::output::{AssigneeQueryResponse, JsonOutput};

                    let response = AssigneeQueryResponse {
                        assignee: assignee.clone(),
                        issues: issues.clone(),
                        count: issues.len(),
                    };
                    let output = JsonOutput::success(response);
                    println!("{}", output.to_json_string()?);
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
            jit::cli::QueryCommands::State { state, json } => match parse_state(&state) {
                Ok(parsed_state) => {
                    let issues = executor.query_by_state(parsed_state)?;
                    if json {
                        use jit::output::{JsonOutput, StateQueryResponse};

                        let response = StateQueryResponse {
                            state: parsed_state,
                            issues: issues.clone(),
                            count: issues.len(),
                        };
                        let output = JsonOutput::success(response);
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!("Issues with state '{}':", state);
                        for issue in &issues {
                            println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                        }
                        println!("\nTotal: {}", issues.len());
                    }
                }
                Err(e) => {
                    if json {
                        use jit::output::JsonError;
                        let json_error = JsonError::invalid_state(&state);
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(json_error.exit_code().code());
                    } else {
                        return Err(e);
                    }
                }
            },
            jit::cli::QueryCommands::Priority { priority, json } => match parse_priority(&priority)
            {
                Ok(parsed_priority) => {
                    let issues = executor.query_by_priority(parsed_priority)?;
                    if json {
                        use jit::output::{JsonOutput, PriorityQueryResponse};

                        let response = PriorityQueryResponse {
                            priority: parsed_priority,
                            issues: issues.clone(),
                            count: issues.len(),
                        };
                        let output = JsonOutput::success(response);
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!("Issues with priority '{}':", priority);
                        for issue in &issues {
                            println!("  {} | {} | {:?}", issue.id, issue.title, issue.state);
                        }
                        println!("\nTotal: {}", issues.len());
                    }
                }
                Err(e) => {
                    if json {
                        use jit::output::JsonError;
                        let json_error = JsonError::invalid_priority(&priority);
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(json_error.exit_code().code());
                    } else {
                        return Err(e);
                    }
                }
            },
            jit::cli::QueryCommands::Label { pattern, json } => {
                match executor.query_by_label(&pattern) {
                    Ok(issues) => {
                        if json {
                            use jit::output::{JsonOutput, LabelQueryResponse};
                            let response = LabelQueryResponse {
                                pattern: pattern.clone(),
                                issues: issues.clone(),
                                count: issues.len(),
                            };
                            let output = JsonOutput::success(response);
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("Issues matching label '{}':", pattern);
                            for issue in &issues {
                                println!("  {} | {} | {:?}", issue.id, issue.title, issue.state);
                            }
                            println!("\nTotal: {}", issues.len());
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error = JsonError::new("INVALID_LABEL_PATTERN", e.to_string())
                                .with_suggestion("Use 'namespace:value' for exact match or 'namespace:*' for wildcard");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            jit::cli::QueryCommands::Strategic { json } => match executor.query_strategic() {
                Ok(issues) => {
                    if json {
                        use jit::output::{JsonOutput, StrategicQueryResponse};
                        let response = StrategicQueryResponse {
                            issues: issues.clone(),
                            count: issues.len(),
                        };
                        let output = JsonOutput::success(response);
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!("Strategic issues:");
                        for issue in &issues {
                            println!("  {} | {} | {:?}", issue.id, issue.title, issue.state);
                        }
                        println!("\nTotal: {}", issues.len());
                    }
                }
                Err(e) => {
                    if json {
                        use jit::output::JsonError;
                        let json_error = JsonError::new("QUERY_FAILED", e.to_string());
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(json_error.exit_code().code());
                    } else {
                        return Err(e);
                    }
                }
            },
        },
        Commands::Label(label_cmd) => match label_cmd {
            jit::cli::LabelCommands::Namespaces { json } => {
                let namespaces = executor.storage().load_label_namespaces()?;
                if json {
                    use jit::output::JsonOutput;
                    let output = JsonOutput::success(namespaces);
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Label Namespaces:\n");
                    for (name, ns) in &namespaces.namespaces {
                        println!("  {}", name);
                        println!("    Description: {}", ns.description);
                        println!("    Unique: {}", ns.unique);
                        println!("    Strategic: {}", ns.strategic);
                        println!();
                    }
                }
            }
            jit::cli::LabelCommands::Values { namespace, json } => {
                let values = executor.list_label_values(&namespace)?;
                if json {
                    use jit::output::JsonOutput;
                    let output = JsonOutput::success(serde_json::json!({
                        "namespace": namespace,
                        "values": values,
                        "count": values.len()
                    }));
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Values in namespace '{}':\n", namespace);
                    for value in &values {
                        println!("  {}", value);
                    }
                    println!("\nTotal: {}", values.len());
                }
            }
            jit::cli::LabelCommands::AddNamespace {
                name,
                description,
                unique,
                strategic,
                json,
            } => {
                executor.add_label_namespace(&name, &description, unique, strategic)?;
                if json {
                    use jit::output::JsonOutput;
                    let output = JsonOutput::success(serde_json::json!({
                        "namespace": name,
                        "description": description,
                        "unique": unique,
                        "strategic": strategic
                    }));
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Added label namespace '{}'", name);
                }
            }
        },
        Commands::Config(config_cmd) => match config_cmd {
            jit::cli::ConfigCommands::ShowHierarchy { json } => {
                let namespaces = executor.storage().load_label_namespaces()?;
                let hierarchy = namespaces.get_type_hierarchy();

                if json {
                    use jit::output::JsonOutput;
                    println!("{}", JsonOutput::success(hierarchy).to_json_string()?);
                } else {
                    println!("Type Hierarchy:\n");
                    let mut sorted: Vec<_> = hierarchy.iter().collect();
                    sorted.sort_by_key(|(_, level)| *level);
                    for (type_name, level) in sorted {
                        println!("  {} → Level {}", type_name, level);
                    }
                }
            }
            jit::cli::ConfigCommands::ListTemplates { json } => {
                let templates = jit::hierarchy_templates::HierarchyTemplate::all();

                if json {
                    use jit::output::JsonOutput;
                    use serde_json::json;
                    let template_data: Vec<_> = templates
                        .iter()
                        .map(|t| {
                            json!({
                                "name": t.name,
                                "description": t.description,
                                "hierarchy": t.hierarchy
                            })
                        })
                        .collect();
                    println!("{}", JsonOutput::success(template_data).to_json_string()?);
                } else {
                    println!("Available Hierarchy Templates:\n");
                    for template in templates {
                        println!("  {}", template.name);
                        println!("    {}", template.description);
                        println!();
                    }
                }
            }
        },
        Commands::Search {
            query,
            regex,
            case_sensitive,
            context,
            limit,
            glob,
            json,
        } => {
            use jit::search::{search, SearchOptions};

            let options = SearchOptions {
                case_sensitive,
                regex,
                context_lines: context,
                max_results: limit,
                file_pattern: glob.clone(),
                file_patterns: Vec::new(),
            };

            match search(&jit_dir, &query, options) {
                Ok(results) => {
                    if json {
                        use jit::output::JsonOutput;
                        use serde_json::json;

                        let output = JsonOutput::success(json!({
                            "query": query,
                            "total": results.len(),
                            "results": results
                        }));
                        println!("{}", output.to_json_string()?);
                    } else if results.is_empty() {
                        println!("No matches found for \"{}\"", query);
                    } else {
                        println!(
                            "Search results for \"{}\" ({} matches):\n",
                            query,
                            results.len()
                        );

                        let mut current_file = String::new();
                        for result in &results {
                            if result.path != current_file {
                                current_file = result.path.clone();

                                if let Some(issue_id) = &result.issue_id {
                                    // Try to load issue for title
                                    if let Ok(issue) = storage.load_issue(issue_id) {
                                        println!("Issue {} | {}", issue_id, issue.title);
                                    } else {
                                        println!("Issue {}", issue_id);
                                    }
                                } else {
                                    println!("Document {}", result.path);
                                }
                            }

                            println!("  Line {}: {}", result.line_number, result.line_text.trim());
                        }
                        println!();
                    }
                }
                Err(e) => {
                    if json {
                        use jit::output::JsonError;

                        let error_code = if e.to_string().contains("not installed") {
                            "RIPGREP_NOT_FOUND"
                        } else {
                            "SEARCH_FAILED"
                        };

                        let suggestion = if error_code == "RIPGREP_NOT_FOUND" {
                            Some(
                                "Install ripgrep from https://github.com/BurntSushi/ripgrep"
                                    .to_string(),
                            )
                        } else {
                            None
                        };

                        let mut json_error = JsonError::new(error_code, e.to_string());
                        if let Some(sug) = suggestion {
                            json_error = json_error.with_suggestion(sug);
                        }
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(10); // External dependency failed
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        Commands::Status { json } => {
            if json {
                use jit::output::JsonOutput;

                let summary = executor.get_status()?;
                let output = JsonOutput::success(&summary);
                println!("{}", output.to_json_string()?);
            } else {
                executor.status()?;
            }
        }
        Commands::Validate { json, fix, dry_run } => {
            // Validate dry_run requires fix
            if dry_run && !fix {
                return Err(anyhow!("--dry-run requires --fix to be specified"));
            }

            if fix {
                // Use auto-fix mode (pass quiet=true if json mode)
                let fixes_applied = executor.validate_with_fix(true, dry_run, json)?;

                if json {
                    use jit::output::JsonOutput;
                    use serde_json::json;

                    let output = JsonOutput::success(json!({
                        "valid": true,
                        "fixes_applied": fixes_applied,
                        "dry_run": dry_run,
                        "message": if dry_run {
                            format!("{} fixes would be applied", fixes_applied)
                        } else if fixes_applied > 0 {
                            format!("Applied {} fixes, repository is now valid", fixes_applied)
                        } else {
                            "Repository is valid".to_string()
                        }
                    }));
                    println!("{}", output.to_json_string()?);
                }
            } else {
                // Standard validation with warnings
                executor.validate_silent()?;
                let warnings = executor.collect_all_warnings()?;

                if json {
                    use jit::output::JsonOutput;
                    use jit::type_hierarchy::ValidationWarning;
                    use serde_json::json;

                    let warnings_json: Vec<_> = warnings
                        .iter()
                        .flat_map(|(issue_id, issue_warnings)| {
                            issue_warnings.iter().map(move |w| match w {
                                ValidationWarning::MissingStrategicLabel {
                                    type_name,
                                    expected_namespace,
                                    ..
                                } => {
                                    json!({
                                        "type": "missing_strategic_label",
                                        "issue_id": issue_id,
                                        "issue_type": type_name,
                                        "expected_namespace": expected_namespace,
                                        "suggestion": format!("Add label: {}:*", expected_namespace)
                                    })
                                }
                                ValidationWarning::OrphanedLeaf { type_name, .. } => {
                                    json!({
                                        "type": "orphaned_leaf",
                                        "issue_id": issue_id,
                                        "issue_type": type_name,
                                        "suggestion": "Add label: epic:* or milestone:*"
                                    })
                                }
                            })
                        })
                        .collect();

                    let output = JsonOutput::success(json!({
                        "valid": true,
                        "warnings": warnings_json,
                        "warning_count": warnings_json.len(),
                        "message": "Repository validation passed"
                    }));
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("✓ Repository validation passed");

                    if !warnings.is_empty() {
                        use jit::type_hierarchy::ValidationWarning;

                        println!(
                            "\nWarnings: {}",
                            warnings.iter().map(|(_, w)| w.len()).sum::<usize>()
                        );
                        println!();

                        for (issue_id, issue_warnings) in warnings {
                            for warning in issue_warnings {
                                match warning {
                                    ValidationWarning::MissingStrategicLabel {
                                        type_name,
                                        expected_namespace,
                                        ..
                                    } => {
                                        println!(
                                            "⚠ Issue {} (type:{}): Missing {}:* label",
                                            issue_id, type_name, expected_namespace
                                        );
                                        println!(
                                            "  Suggested: jit issue update {} --label \"{}:value\"",
                                            issue_id, expected_namespace
                                        );
                                    }
                                    ValidationWarning::OrphanedLeaf { type_name, .. } => {
                                        println!(
                                            "⚠ Issue {} (type:{}): Orphaned leaf issue",
                                            issue_id, type_name
                                        );
                                        println!("  Suggested: jit issue update {} --label \"epic:value\"", 
                                                issue_id);
                                    }
                                }
                                println!();
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
