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

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use jit::cli::{
    Cli, Commands, DepCommands, DocCommands, EventCommands, GateCommands, GraphCommands,
    IssueCommands, RegistryCommands,
};
use jit::commands::{parse_priority, parse_state, CommandExecutor};
use jit::output::{ExitCode, JsonOutput, OutputContext};
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
    } else if error_msg.contains("gate validation failed") {
        // Gate blocking should return validation failed
        ExitCode::ValidationFailed
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
    let quiet = cli.quiet;

    // Handle --schema flag first
    if cli.schema {
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
            let output_ctx = OutputContext::new(quiet, false);
            executor.init()?;

            // If a template is specified, write it to config.toml
            if let Some(template_name) = hierarchy_template {
                let template = jit::hierarchy_templates::HierarchyTemplate::get(template_name)
                    .ok_or_else(|| anyhow!("Unknown hierarchy template: {}", template_name))?;

                // Generate config.toml content with the template
                let config_path = jit_dir.join("config.toml");
                let config_content = format!(
                    r#"[version]
schema = 2

[type_hierarchy]
types = {}
strategic_types = {}

[type_hierarchy.label_associations]
{}

# Add namespace definitions below
# [namespaces.type]
# description = "Issue type (hierarchical)"
# unique = true
"#,
                    toml::to_string(&template.hierarchy)?,
                    toml::to_string(&template.get_strategic_types())?,
                    template
                        .label_associations
                        .iter()
                        .map(|(k, v)| format!("{} = \"{}\"", k, v))
                        .collect::<Vec<_>>()
                        .join("\n")
                );

                std::fs::write(&config_path, config_content)?;

                let _ = output_ctx.print_success(format!(
                    "Initialized with '{}' hierarchy template",
                    template_name
                ));
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
                    description,
                    priority,
                    gate,
                    label,
                    force,
                    orphan,
                    json,
                } => {
                    let prio = parse_priority(&priority)?;
                    let id = executor.create_issue(title, description, prio, gate, label)?;
                    let output_ctx = OutputContext::new(quiet, json);

                    if json {
                        let issue = storage.load_issue(&id)?;
                        let output = JsonOutput::success(issue, "issue create");
                        println!("{}", output.to_json_string()?);
                    } else {
                        // In quiet mode, output just the ID for scripting
                        if quiet {
                            println!("{}", id);
                        } else {
                            println!("Created issue: {}", id);
                        }

                        // Check for warnings unless --force or --quiet
                        if !force && !quiet {
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
                                        let _ = output_ctx.print_warning(format!("\n⚠ Strategic consistency issue\n  Issue {} (type:{}) should have a {}:* label for identification.\n  Suggested: jit issue update {} --label \"{}:value\"",
                                            id, type_name, expected_namespace, id, expected_namespace));
                                    }
                                    ValidationWarning::OrphanedLeaf { type_name, .. } => {
                                        let _ = output_ctx.print_warning(format!("\n⚠ Orphaned leaf issue\n  {} {} has no parent association (epic or milestone).\n  Consider adding: --label \"epic:value\" or --label \"milestone:value\"\n  Or use --orphan flag to acknowledge intentional orphan.",
                                            type_name.to_uppercase(), id));
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

                        let output = JsonOutput::success(
                            json!({
                                "issues": issues,
                                "summary": {
                                    "total": issues.len(),
                                    "by_state": state_counts,
                                }
                            }),
                            "issue list",
                        );
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
                    let output_ctx = OutputContext::new(quiet, json);
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

                        let output = JsonOutput::success(
                            json!({
                                "query": query,
                                "issues": issues,
                                "count": issues.len(),
                            }),
                            "issue search",
                        );
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_info(format!("Found {} issue(s):", issues.len()));
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
                        output_data!(quiet, json, "issue show", issue, {
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
                        handle_json_error!(
                            json,
                            e,
                            jit::output::JsonError::issue_not_found(&id, "issue show")
                        );
                    }
                },
                IssueCommands::Update {
                    id,
                    filter,
                    title,
                    description,
                    priority,
                    state,
                    label,
                    remove_label,
                    add_gate,
                    remove_gate,
                    assignee,
                    unassign,
                    json,
                } => {
                    let output_ctx = OutputContext::new(quiet, json);

                    // Validate: exactly one of ID or filter must be provided
                    if id.is_none() && filter.is_none() {
                        return Err(anyhow!(
                            "Must specify either issue ID or --filter for batch mode"
                        ));
                    }
                    if id.is_some() && filter.is_some() {
                        return Err(anyhow!(
                            "Cannot specify both ID and --filter (mutually exclusive)"
                        ));
                    }

                    // Single issue mode
                    if let Some(id_str) = id {
                        // Resolve short hash to full UUID first
                        let full_id = storage.resolve_issue_id(&id_str)?;

                        let prio = priority.map(|p| parse_priority(&p)).transpose()?;
                        let st = state.map(|s| parse_state(&s)).transpose()?;

                        // Handle gate modifications first (before other updates)
                        if !add_gate.is_empty() {
                            executor.add_gates(&full_id, &add_gate)?;
                        }

                        if !remove_gate.is_empty() {
                            executor.remove_gates(&full_id, &remove_gate)?;
                        }

                        match executor.update_issue(
                            &full_id,
                            title,
                            description,
                            prio,
                            st,
                            label,
                            remove_label,
                        ) {
                            Ok(()) => {
                                if json {
                                    let issue = storage.load_issue(&full_id)?;
                                    let output = JsonOutput::success(issue, "issue update");
                                    println!("{}", output.to_json_string()?);
                                } else {
                                    let _ = output_ctx
                                        .print_success(format!("Updated issue: {}", full_id));
                                }
                            }
                            Err(e) => {
                                // Check if this is a gate validation error
                                let error_msg = e.to_string();
                                let json_error = if error_msg.contains("Gate validation failed") {
                                    // Reload issue to get unpassed gates
                                    let issue = storage.load_issue(&full_id)?;
                                    let unpassed = issue.get_unpassed_gates();
                                    jit::output::JsonError::gate_validation_failed(
                                        &unpassed,
                                        &full_id,
                                        "issue update",
                                    )
                                } else if error_msg.contains("not found") {
                                    jit::output::JsonError::issue_not_found(
                                        &full_id,
                                        "issue update",
                                    )
                                } else {
                                    // Generic error - use the JsonError::new directly
                                    jit::output::JsonError::new(
                                        "GENERIC_ERROR",
                                        &error_msg,
                                        "issue update",
                                    )
                                };
                                handle_json_error!(json, e, json_error);
                            }
                        }
                    }
                    // Batch mode
                    else if let Some(filter_str) = filter {
                        use jit::commands::bulk_update::UpdateOperations;
                        use jit::query::QueryFilter;

                        // Parse query filter
                        let query_filter = QueryFilter::parse(&filter_str)?;

                        // Build update operations
                        let operations = UpdateOperations {
                            state: state.map(|s| parse_state(&s)).transpose()?,
                            add_labels: label,
                            remove_labels: remove_label,
                            assignee,
                            unassign,
                            priority: priority.map(|p| parse_priority(&p)).transpose()?,
                            add_gates: add_gate,
                            remove_gates: remove_gate,
                        };

                        // Execute bulk update
                        let result = executor.apply_bulk_update(&query_filter, &operations)?;

                        if json {
                            let output = JsonOutput::success(result, "bulk update");
                            println!("{}", output.to_json_string()?);
                        } else {
                            // Human-readable output
                            if result.summary.total_modified > 0 {
                                let _ = output_ctx.print_success(format!(
                                    "✓ Modified {} issue(s)",
                                    result.summary.total_modified
                                ));
                            }

                            if !result.skipped.is_empty() {
                                println!("\nℹ Skipped {} issue(s):", result.summary.total_skipped);
                                for (id, reason) in &result.skipped {
                                    println!("  • {}: {}", &id[..8.min(id.len())], reason);
                                }
                            }

                            if !result.errors.is_empty() {
                                println!("\n✗ Failed {} issue(s):", result.summary.total_errors);
                                for (id, error) in &result.errors {
                                    println!("  • {}: {}", &id[..8.min(id.len())], error);
                                }
                            }

                            if result.summary.total_matched > 0 {
                                println!(
                                    "\nSummary: {}/{} succeeded ({:.0}%)",
                                    result.summary.total_modified,
                                    result.summary.total_matched,
                                    (result.summary.total_modified as f64
                                        / result.summary.total_matched as f64)
                                        * 100.0
                                );
                            } else {
                                println!("No issues matched filter");
                            }
                        }
                    }
                }
                IssueCommands::Delete { id, json } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    executor.delete_issue(&id)?;

                    if json {
                        let result = serde_json::json!({
                            "id": id,
                            "deleted": true
                        });
                        let output = JsonOutput::success(result, "issue delete");
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Deleted issue: {}", id));
                    }
                }
                IssueCommands::Breakdown {
                    parent_id,
                    subtask_titles,
                    subtask_descriptions,
                    json,
                } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    // Pad descriptions with empty strings if not enough provided
                    let mut descs = subtask_descriptions.clone();
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
                        let output = JsonOutput::success(response, "issue breakdown");
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_info(format!(
                            "Broke down {} into {} subtasks:",
                            parent_id,
                            subtask_ids.len()
                        ));
                        for (i, id) in subtask_ids.iter().enumerate() {
                            println!("  {}. {}", i + 1, id);
                        }
                    }
                }
                IssueCommands::Assign { id, assignee, json } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let full_id = storage.resolve_issue_id(&id)?;
                    executor.assign_issue(&full_id, assignee)?;

                    if json {
                        let issue = storage.load_issue(&full_id)?;
                        let output = JsonOutput::success(issue, "issue assign");
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Assigned issue: {}", full_id));
                    }
                }
                IssueCommands::Claim { id, assignee, json } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let full_id = storage.resolve_issue_id(&id)?;
                    executor.claim_issue(&full_id, assignee)?;

                    if json {
                        let issue = storage.load_issue(&full_id)?;
                        let output = JsonOutput::success(issue, "issue claim");
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Claimed issue: {}", full_id));
                    }
                }
                IssueCommands::Unassign { id, json } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let full_id = storage.resolve_issue_id(&id)?;
                    executor.unassign_issue(&full_id)?;

                    if json {
                        let issue = storage.load_issue(&full_id)?;
                        let output = JsonOutput::success(issue, "issue unassign");
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Unassigned issue: {}", full_id));
                    }
                }
                IssueCommands::Reject { id, reason, json } => {
                    use jit::domain::State;

                    let output_ctx = OutputContext::new(quiet, json);
                    let full_id = storage.resolve_issue_id(&id)?;

                    // Update state to rejected
                    executor.update_issue_state(&full_id, State::Rejected)?;

                    // Add resolution label if reason provided
                    if let Some(ref reason_value) = reason {
                        let label = format!("resolution:{}", reason_value);
                        executor.add_label(&full_id, &label)?;
                    }

                    if json {
                        let issue = storage.load_issue(&full_id)?;
                        let output = JsonOutput::success(issue, "issue reject");
                        println!("{}", output.to_json_string()?);
                    } else if let Some(reason_value) = reason {
                        let _ = output_ctx.print_success(format!(
                            "Rejected issue: {} (reason: {})",
                            full_id, reason_value
                        ));
                    } else {
                        let _ = output_ctx.print_success(format!("Rejected issue: {}", full_id));
                    }
                }
                IssueCommands::Release { id, reason, json } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let full_id = storage.resolve_issue_id(&id)?;
                    executor.release_issue(&full_id, &reason)?;

                    if json {
                        let issue = storage.load_issue(&full_id)?;
                        let output = JsonOutput::success(issue, "issue release");
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!(
                            "Released issue: {} (reason: {})",
                            full_id, reason
                        ));
                    }
                }
                IssueCommands::ClaimNext { assignee, filter } => {
                    let output_ctx = OutputContext::new(quiet, false);
                    let id = executor.claim_next(assignee, filter)?;
                    let _ = output_ctx.print_success(format!("Claimed issue: {}", id));
                }
            }
        }
        Commands::Dep(dep_cmd) => match dep_cmd {
            DepCommands::Add {
                from_id,
                to_ids,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.add_dependencies(&from_id, &to_ids) {
                    Ok(result) => {
                        // If all dependencies failed, return error
                        if result.added.is_empty() && !result.errors.is_empty() {
                            if json {
                                use jit::output::JsonError;
                                // Get first error for backward compatibility
                                let (dep_id, error_msg) = &result.errors[0];
                                let json_error = if error_msg.contains("cycle") {
                                    JsonError::cycle_detected(&from_id, dep_id, "dep add")
                                } else if error_msg.contains("not found") {
                                    JsonError::issue_not_found(dep_id, "dep add")
                                } else {
                                    JsonError::new("DEPENDENCY_ERROR", error_msg.clone(), "dep add")
                                };
                                println!("{}", json_error.to_json_string()?);
                                std::process::exit(json_error.exit_code().code());
                            } else {
                                return Err(anyhow!("{}", result.errors[0].1));
                            }
                        }

                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "from_id": from_id,
                                "added": result.added,
                                "already_exist": result.already_exist,
                                "skipped": result.skipped,
                                "errors": result.errors,
                                "message": format!("Added {} dependencies to issue {}", result.added.len(), from_id)
                            });
                            let output = JsonOutput::success(response, "dep add");
                            println!("{}", output.to_json_string()?);
                        } else {
                            if !result.added.is_empty() {
                                let _ = output_ctx.print_success(format!(
                                    "Added {} dependenc{}:",
                                    result.added.len(),
                                    if result.added.len() == 1 { "y" } else { "ies" }
                                ));
                                for dep in &result.added {
                                    println!("  • {} → {}", from_id, dep);
                                }
                            }
                            if !result.already_exist.is_empty() {
                                println!("ℹ Already exist ({}):", result.already_exist.len());
                                for dep in &result.already_exist {
                                    println!("  • {}", dep);
                                }
                            }
                            if !result.skipped.is_empty() {
                                println!("ℹ Skipped ({}):", result.skipped.len());
                                for (dep, reason) in &result.skipped {
                                    println!("  • {}: {}", dep, reason);
                                }
                            }
                            if !result.errors.is_empty() {
                                println!("✗ Errors ({}):", result.errors.len());
                                for (dep, error) in &result.errors {
                                    println!("  • {}: {}", dep, error);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let error_str = e.to_string();
                            let json_error =
                                JsonError::new("DEPENDENCY_ERROR", error_str, "dep add");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            DepCommands::Rm {
                from_id,
                to_ids,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.remove_dependencies(&from_id, &to_ids) {
                    Ok(result) => {
                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "from_id": from_id,
                                "removed": result.removed,
                                "not_found": result.not_found,
                                "message": format!("Removed {} dependencies from issue {}", result.removed.len(), from_id)
                            });
                            let output = JsonOutput::success(response, "dep rm");
                            println!("{}", output.to_json_string()?);
                        } else {
                            if !result.removed.is_empty() {
                                let _ = output_ctx.print_success(format!(
                                    "Removed {} dependenc{}:",
                                    result.removed.len(),
                                    if result.removed.len() == 1 {
                                        "y"
                                    } else {
                                        "ies"
                                    }
                                ));
                                for dep in &result.removed {
                                    println!("  • {}", dep);
                                }
                            }
                            if !result.not_found.is_empty() {
                                println!("ℹ Not found ({}):", result.not_found.len());
                                for dep in &result.not_found {
                                    println!("  • {}", dep);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let error_str = e.to_string();
                            let json_error =
                                JsonError::new("DEPENDENCY_ERROR", error_str, "dep rm");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
        },
        Commands::Gate(gate_cmd) => match gate_cmd {
            GateCommands::Define {
                key,
                title,
                description,
                stage,
                mode,
                checker_command,
                timeout,
                working_dir,
                json,
            } => {
                use jit::domain::{GateChecker, GateMode, GateStage};

                let output_ctx = OutputContext::new(quiet, json);

                // Parse stage
                let gate_stage = match stage.to_lowercase().as_str() {
                    "precheck" => GateStage::Precheck,
                    "postcheck" => GateStage::Postcheck,
                    _ => {
                        eprintln!(
                            "Error: Invalid stage '{}'. Use 'precheck' or 'postcheck'",
                            stage
                        );
                        std::process::exit(2);
                    }
                };

                // Parse mode
                let gate_mode = match mode.to_lowercase().as_str() {
                    "manual" => GateMode::Manual,
                    "auto" => GateMode::Auto,
                    _ => {
                        eprintln!("Error: Invalid mode '{}'. Use 'manual' or 'auto'", mode);
                        std::process::exit(2);
                    }
                };

                // Build checker if command provided
                let checker = checker_command.map(|cmd| GateChecker::Exec {
                    command: cmd,
                    timeout_seconds: timeout,
                    working_dir: working_dir.clone(),
                    env: std::collections::HashMap::new(),
                });

                match executor.define_gate(
                    key.clone(),
                    title.clone(),
                    description.clone(),
                    gate_stage,
                    gate_mode,
                    checker,
                ) {
                    Ok(_) => {
                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "gate_key": key,
                                "message": format!("Defined gate '{}'", key)
                            });
                            let output = JsonOutput::success(response, "gate define");
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx.print_success(format!("Defined gate '{}'", key));
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error = JsonError::new("GATE_ERROR", e.to_string(), "gate");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::List { json } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.list_gates() {
                    Ok(gates) => {
                        if json {
                            use jit::output::JsonOutput;
                            let output = JsonOutput::success(gates, "gate list");
                            println!("{}", output.to_json_string()?);
                        } else if gates.is_empty() {
                            let _ = output_ctx.print_info("No gates defined");
                        } else {
                            let _ = output_ctx.print_info("Gates:");
                            for gate in gates {
                                println!(
                                    "  {} - {} ({:?}, {:?})",
                                    gate.key, gate.title, gate.stage, gate.mode
                                );
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error = JsonError::new("GATE_ERROR", e.to_string(), "gate");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::Show { key, json } => match executor.show_gate_definition(&key) {
                Ok(gate) => {
                    if json {
                        use jit::output::JsonOutput;
                        let output = JsonOutput::success(gate, "gate show");
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!("Gate: {}", gate.key);
                        println!("  Title: {}", gate.title);
                        println!("  Description: {}", gate.description);
                        println!("  Stage: {:?}", gate.stage);
                        println!("  Mode: {:?}", gate.mode);
                        if let Some(checker) = gate.checker {
                            match checker {
                                jit::domain::GateChecker::Exec {
                                    command,
                                    timeout_seconds,
                                    working_dir,
                                    ..
                                } => {
                                    println!("  Checker:");
                                    println!("    Command: {}", command);
                                    println!("    Timeout: {}s", timeout_seconds);
                                    if let Some(wd) = working_dir {
                                        println!("    Working dir: {}", wd);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    if json {
                        use jit::output::JsonError;
                        let json_error = JsonError::gate_not_found(&key, "gate show");
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(json_error.exit_code().code());
                    } else {
                        return Err(e);
                    }
                }
            },
            GateCommands::Remove { key, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.remove_gate_definition(&key) {
                    Ok(_) => {
                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "gate_key": key,
                                "message": format!("Removed gate '{}'", key)
                            });
                            let output = JsonOutput::success(response, "gate remove");
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx.print_success(format!("Removed gate '{}'", key));
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error = JsonError::gate_not_found(&key, "gate show");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::Check { id, gate_key, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.check_gate(&id, &gate_key) {
                    Ok(result) => {
                        if json {
                            use jit::output::JsonOutput;
                            let output = JsonOutput::success(result, "gate check");
                            println!("{}", output.to_json_string()?);
                        } else {
                            match result.status {
                                jit::domain::GateRunStatus::Passed => {
                                    let _ = output_ctx.print_success(format!(
                                        "✓ Gate '{}' passed for issue {}",
                                        gate_key, id
                                    ));
                                }
                                jit::domain::GateRunStatus::Failed => {
                                    println!("✗ Gate '{}' failed for issue {}", gate_key, id);
                                    if !result.stderr.is_empty() {
                                        eprintln!(
                                            "  Error: {}",
                                            result.stderr.lines().next().unwrap_or("")
                                        );
                                    }
                                }
                                jit::domain::GateRunStatus::Error => {
                                    println!("✗ Gate '{}' error for issue {}", gate_key, id);
                                    eprintln!("  {}", result.stderr);
                                }
                                _ => {
                                    println!("Gate '{}' status: {:?}", gate_key, result.status);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error =
                                JsonError::new("GATE_CHECK_ERROR", e.to_string(), "gate check");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::CheckAll { id, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.check_all_gates(&id) {
                    Ok(results) => {
                        if json {
                            use jit::output::JsonOutput;
                            let output = JsonOutput::success(results, "gate check-all");
                            println!("{}", output.to_json_string()?);
                        } else if results.is_empty() {
                            let _ = output_ctx.print_info(format!(
                                "No automated gates to check for issue {}",
                                id
                            ));
                        } else {
                            let _ =
                                output_ctx.print_info(format!("Checking gates for issue {}:", id));
                            for result in results {
                                match result.status {
                                    jit::domain::GateRunStatus::Passed => {
                                        println!("  ✓ {} passed", result.gate_key);
                                    }
                                    jit::domain::GateRunStatus::Failed => {
                                        println!("  ✗ {} failed", result.gate_key);
                                    }
                                    _ => {
                                        println!("  {} - {:?}", result.gate_key, result.status);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error =
                                JsonError::new("GATE_CHECK_ERROR", e.to_string(), "gate check");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::Add {
                id,
                gate_keys,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.add_gates(&id, &gate_keys) {
                    Ok(result) => {
                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "issue_id": id,
                                "added": result.added,
                                "already_exist": result.already_exist,
                                "message": format!("Added {} gate(s) to issue {}", result.added.len(), id)
                            });
                            let output = JsonOutput::success(response, "gate add");
                            println!("{}", output.to_json_string()?);
                        } else {
                            if !result.added.is_empty() {
                                let _ = output_ctx.print_success(format!(
                                    "Added {} gate(s) to issue {}:",
                                    result.added.len(),
                                    id
                                ));
                                for gate in &result.added {
                                    println!("  • {}", gate);
                                }
                            }
                            if !result.already_exist.is_empty() {
                                println!(
                                    "ℹ Already required ({} gate(s)):",
                                    result.already_exist.len()
                                );
                                for gate in &result.already_exist {
                                    println!("  • {}", gate);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let error_str = e.to_string();
                            let json_error =
                                if error_str.contains("Issue") && error_str.contains("not found") {
                                    JsonError::issue_not_found(&id, "gate add")
                                } else if error_str.contains("not found in registry") {
                                    JsonError::new("GATE_NOT_FOUND", error_str, "gate add")
                                } else {
                                    JsonError::new("GATE_ERROR", error_str, "gate add")
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
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.pass_gate(&id, gate_key.clone(), by) {
                    Ok(_) => {
                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "issue_id": id,
                                "gate_key": gate_key,
                                "status": "passed",
                                "message": format!("Passed gate '{}' for issue {}", gate_key, id)
                            });
                            let output = JsonOutput::success(response, "gate pass");
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx.print_success(format!(
                                "Passed gate '{}' for issue {}",
                                gate_key, id
                            ));
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error =
                                JsonError::new("GATE_ERROR", e.to_string(), "gate pass");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::Fail {
                id,
                gate_key,
                by,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.fail_gate(&id, gate_key.clone(), by) {
                    Ok(_) => {
                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "issue_id": id,
                                "gate_key": gate_key,
                                "status": "failed",
                                "message": format!("Failed gate '{}' for issue {}", gate_key, id)
                            });
                            let output = JsonOutput::success(response, "gate fail");
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx.print_success(format!(
                                "Failed gate '{}' for issue {}",
                                gate_key, id
                            ));
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error =
                                JsonError::new("GATE_ERROR", e.to_string(), "gate fail");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
        },
        Commands::Graph(graph_cmd) => match graph_cmd {
            GraphCommands::Show { id, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                if let Some(issue_id) = id {
                    let issues = executor.show_graph(&issue_id)?;
                    if json {
                        use jit::output::{GraphShowResponse, JsonOutput};

                        let response = GraphShowResponse {
                            issue_id: issue_id.clone(),
                            dependencies: issues.clone(),
                            count: issues.len(),
                        };
                        let output = JsonOutput::success(response, "graph show");
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_info(format!("Dependency tree for {}:", issue_id));
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
                        let output = JsonOutput::success(response, "graph show");
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_info("All dependencies:");
                        for issue in all_issues {
                            if !issue.dependencies.is_empty() {
                                println!("  {} depends on: {:?}", issue.id, issue.dependencies);
                            }
                        }
                    }
                }
            }
            GraphCommands::Downstream { id, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let issues = executor.show_downstream(&id)?;
                if json {
                    use jit::output::{GraphDownstreamResponse, JsonOutput};

                    let response = GraphDownstreamResponse {
                        issue_id: id.clone(),
                        dependents: issues.clone(),
                        count: issues.len(),
                    };
                    let output = JsonOutput::success(response, "graph downstream");
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info(format!("Downstream dependents of {}:", id));
                    for issue in issues {
                        println!("  {} | {}", issue.id, issue.title);
                    }
                }
            }
            GraphCommands::Roots { json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let issues = executor.show_roots()?;
                if json {
                    use jit::output::{GraphRootsResponse, JsonOutput};

                    let response = GraphRootsResponse {
                        roots: issues.clone(),
                        count: issues.len(),
                    };
                    let output = JsonOutput::success(response, "graph roots");
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info("Root issues (no dependencies):");
                    for issue in issues {
                        println!("  {} | {}", issue.id, issue.title);
                    }
                }
            }
            GraphCommands::Export { format, output } => {
                let output_ctx = OutputContext::new(quiet, false);
                let graph_output = executor.export_graph(&format)?;

                if let Some(path) = output {
                    std::fs::write(&path, graph_output)?;
                    let _ = output_ctx.print_success(format!("Graph exported to: {}", path));
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
                            stage: match g.stage {
                                jit::domain::GateStage::Precheck => "precheck".to_string(),
                                jit::domain::GateStage::Postcheck => "postcheck".to_string(),
                            },
                            mode: match g.mode {
                                jit::domain::GateMode::Manual => "manual".to_string(),
                                jit::domain::GateMode::Auto => "auto".to_string(),
                            },
                        })
                        .collect();

                    let response = RegistryListResponse {
                        count: gate_defs.len(),
                        gates: gate_defs,
                    };
                    let output = JsonOutput::success(response, "registry list");
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
                description,
                auto,
                example,
                stage,
            } => {
                let output_ctx = OutputContext::new(quiet, false);
                executor.add_gate_definition(
                    key.clone(),
                    title,
                    description,
                    auto,
                    example,
                    stage,
                )?;
                let _ = output_ctx.print_success(format!("Added gate definition: {}", key));
            }
            RegistryCommands::Remove { key } => {
                let output_ctx = OutputContext::new(quiet, false);
                executor.remove_gate_definition(&key)?;
                let _ = output_ctx.print_success(format!("Removed gate definition: {}", key));
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
                        stage: match gate.stage {
                            jit::domain::GateStage::Precheck => "precheck".to_string(),
                            jit::domain::GateStage::Postcheck => "postcheck".to_string(),
                        },
                        mode: match gate.mode {
                            jit::domain::GateMode::Manual => "manual".to_string(),
                            jit::domain::GateMode::Auto => "auto".to_string(),
                        },
                    };
                    let output = JsonOutput::success(gate_def, "registry show");
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Key: {}", gate.key);
                    println!("Title: {}", gate.title);
                    println!("Description: {}", gate.description);
                    println!("Auto: {}", gate.auto);
                    println!("Example Integration: {:?}", gate.example_integration);
                    println!("Stage: {:?}", gate.stage);
                    println!("Mode: {:?}", gate.mode);
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
                skip_scan,
                json,
            } => {
                executor.add_document_reference(
                    &id,
                    &path,
                    commit.as_deref(),
                    label.as_deref(),
                    doc_type.as_deref(),
                    skip_scan,
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
            DocCommands::Assets { command } => match command {
                jit::cli::AssetCommands::List {
                    id,
                    path,
                    rescan,
                    json,
                } => {
                    executor.list_document_assets(&id, &path, rescan, json)?;
                }
            },
            DocCommands::CheckLinks { scope, json } => {
                let exit_code = executor.check_document_links(&scope, json)?;
                std::process::exit(exit_code);
            }
            DocCommands::Archive {
                path,
                category,
                dry_run,
                force,
            } => {
                executor.archive_document(&path, &category, dry_run, force)?;
            }
        },
        Commands::Query(query_cmd) => match query_cmd {
            jit::cli::QueryCommands::Ready { json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let issues = executor.query_ready()?;
                if json {
                    use jit::output::{JsonOutput, ReadyQueryResponse};

                    let response = ReadyQueryResponse {
                        issues: issues.clone(),
                        count: issues.len(),
                    };
                    let output = JsonOutput::success(response, "registry show");
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info("Ready issues (unassigned, unblocked):");
                    for issue in &issues {
                        println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                    }
                    let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                }
            }
            jit::cli::QueryCommands::Blocked { json } => {
                let output_ctx = OutputContext::new(quiet, json);
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
                    let output = JsonOutput::success(response, "registry show");
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info("Blocked issues:");
                    for (issue, reasons) in &blocked {
                        println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                        for reason in reasons {
                            println!("    - {}", reason);
                        }
                    }
                    let _ = output_ctx.print_info(format!("\nTotal: {}", blocked.len()));
                }
            }
            jit::cli::QueryCommands::Assignee { assignee, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let issues = executor.query_by_assignee(&assignee)?;
                if json {
                    use jit::output::{AssigneeQueryResponse, JsonOutput};

                    let response = AssigneeQueryResponse {
                        assignee: assignee.clone(),
                        issues: issues.clone(),
                        count: issues.len(),
                    };
                    let output = JsonOutput::success(response, "registry show");
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info(format!("Issues assigned to {}:", assignee));
                    for issue in &issues {
                        println!(
                            "  {} | {} | {:?} | {:?}",
                            issue.id, issue.title, issue.state, issue.priority
                        );
                    }
                    let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                }
            }
            jit::cli::QueryCommands::State { state, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                match parse_state(&state) {
                    Ok(parsed_state) => {
                        let issues = executor.query_by_state(parsed_state)?;
                        if json {
                            use jit::output::{JsonOutput, StateQueryResponse};

                            let response = StateQueryResponse {
                                state: parsed_state,
                                issues: issues.clone(),
                                count: issues.len(),
                            };
                            let output = JsonOutput::success(response, "registry show");
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ =
                                output_ctx.print_info(format!("Issues with state '{}':", state));
                            for issue in &issues {
                                println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                            }
                            let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error = JsonError::invalid_state(&state, "query state");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            jit::cli::QueryCommands::Priority { priority, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                match parse_priority(&priority) {
                    Ok(parsed_priority) => {
                        let issues = executor.query_by_priority(parsed_priority)?;
                        if json {
                            use jit::output::{JsonOutput, PriorityQueryResponse};

                            let response = PriorityQueryResponse {
                                priority: parsed_priority,
                                issues: issues.clone(),
                                count: issues.len(),
                            };
                            let output = JsonOutput::success(response, "registry show");
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx
                                .print_info(format!("Issues with priority '{}':", priority));
                            for issue in &issues {
                                println!("  {} | {} | {:?}", issue.id, issue.title, issue.state);
                            }
                            let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error =
                                JsonError::invalid_priority(&priority, "query priority");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            jit::cli::QueryCommands::Label { pattern, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.query_by_label(&pattern) {
                    Ok(issues) => {
                        if json {
                            use jit::output::{JsonOutput, LabelQueryResponse};
                            let response = LabelQueryResponse {
                                pattern: pattern.clone(),
                                issues: issues.clone(),
                                count: issues.len(),
                            };
                            let output = JsonOutput::success(response, "registry show");
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx
                                .print_info(format!("Issues matching label '{}':", pattern));
                            for issue in &issues {
                                println!("  {} | {} | {:?}", issue.id, issue.title, issue.state);
                            }
                            let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error = JsonError::new("INVALID_LABEL_PATTERN", e.to_string(), "query label")
                                .with_suggestion("Use 'namespace:value' for exact match or 'namespace:*' for wildcard");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            jit::cli::QueryCommands::Strategic { json } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.query_strategic() {
                    Ok(issues) => {
                        if json {
                            use jit::output::{JsonOutput, StrategicQueryResponse};
                            let response = StrategicQueryResponse {
                                issues: issues.clone(),
                                count: issues.len(),
                            };
                            let output = JsonOutput::success(response, "registry show");
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx.print_info("Strategic issues:");
                            for issue in &issues {
                                println!("  {} | {} | {:?}", issue.id, issue.title, issue.state);
                            }
                            let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error = JsonError::new("QUERY_FAILED", e.to_string(), "query");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            jit::cli::QueryCommands::Closed { json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let issues = executor.query_closed()?;
                if json {
                    use jit::output::{ClosedQueryResponse, JsonOutput};
                    let response = ClosedQueryResponse {
                        issues: issues.clone(),
                        count: issues.len(),
                    };
                    let output = JsonOutput::success(response, "registry show");
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info("Closed issues (Done or Rejected):");
                    for issue in &issues {
                        println!("  {} | {} | {:?}", issue.id, issue.title, issue.state);
                    }
                    let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                }
            }
        },
        Commands::Label(label_cmd) => match label_cmd {
            jit::cli::LabelCommands::Namespaces { json } => {
                let output_ctx = OutputContext::new(quiet, json);
                use jit::config_manager::ConfigManager;
                let config_mgr = ConfigManager::new(&jit_dir);
                let namespaces = config_mgr.get_namespaces()?;
                if json {
                    use jit::output::JsonOutput;
                    let output = JsonOutput::success(namespaces, "registry show");
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info("Label Namespaces:\n");
                    for (name, ns) in &namespaces.namespaces {
                        println!("  {}", name);
                        println!("    Description: {}", ns.description);
                        println!("    Unique: {}", ns.unique);
                        println!();
                    }
                }
            }
            jit::cli::LabelCommands::Values { namespace, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let values = executor.list_label_values(&namespace)?;
                if json {
                    use jit::output::JsonOutput;
                    let output = JsonOutput::success(
                        serde_json::json!({
                            "namespace": namespace,
                            "values": values,
                            "count": values.len()
                        }),
                        "label values",
                    );
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ =
                        output_ctx.print_info(format!("Values in namespace '{}':\n", namespace));
                    for value in &values {
                        println!("  {}", value);
                    }
                    let _ = output_ctx.print_info(format!("\nTotal: {}", values.len()));
                }
            }
        },
        Commands::Config(config_cmd) => match config_cmd {
            jit::cli::ConfigCommands::ShowHierarchy { json } => {
                let output_ctx = OutputContext::new(quiet, json);
                use jit::config_manager::ConfigManager;
                let config_mgr = ConfigManager::new(&jit_dir);
                let namespaces = config_mgr.get_namespaces()?;
                let hierarchy = namespaces.get_type_hierarchy();

                if json {
                    use jit::output::JsonOutput;
                    println!(
                        "{}",
                        JsonOutput::success(hierarchy, "config show-hierarchy").to_json_string()?
                    );
                } else {
                    let _ = output_ctx.print_info("Type Hierarchy:\n");
                    let mut sorted: Vec<_> = hierarchy.iter().collect();
                    sorted.sort_by_key(|(_, level)| *level);
                    for (type_name, level) in sorted {
                        println!("  {} → Level {}", type_name, level);
                    }
                }
            }
            jit::cli::ConfigCommands::ListTemplates { json } => {
                let output_ctx = OutputContext::new(quiet, json);
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
                    println!(
                        "{}",
                        JsonOutput::success(template_data, "config list-templates")
                            .to_json_string()?
                    );
                } else {
                    let _ = output_ctx.print_info("Available Hierarchy Templates:\n");
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
            let output_ctx = OutputContext::new(quiet, json);
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

                        let output = JsonOutput::success(
                            json!({
                                "query": query,
                                "total": results.len(),
                                "results": results
                            }),
                            "search",
                        );
                        println!("{}", output.to_json_string()?);
                    } else if results.is_empty() {
                        let _ =
                            output_ctx.print_info(format!("No matches found for \"{}\"", query));
                    } else {
                        let _ = output_ctx.print_info(format!(
                            "Search results for \"{}\" ({} matches):\n",
                            query,
                            results.len()
                        ));

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

                        let mut json_error = JsonError::new(error_code, e.to_string(), "validate");
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
                let output = JsonOutput::success(&summary, "status");
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

                    let output = JsonOutput::success(
                        json!({
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
                        }),
                        "validate",
                    );
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

                    let output = JsonOutput::success(
                        json!({
                            "valid": true,
                            "warnings": warnings_json,
                            "warning_count": warnings_json.len(),
                            "message": "Repository validation passed"
                        }),
                        "validate",
                    );
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
        Commands::Snapshot(snapshot_cmd) => match snapshot_cmd {
            jit::cli::SnapshotCommands::Export {
                out,
                format,
                scope,
                at,
                working_tree,
                committed_only,
                force,
                json,
            } => {
                use jit::commands::snapshot::SnapshotExporter;
                use jit::snapshot::{SnapshotFormat, SnapshotScope};

                // Parse scope
                let snapshot_scope = SnapshotScope::parse(&scope)
                    .with_context(|| format!("Invalid scope: {}", scope))?;

                // Parse format
                let snapshot_format = SnapshotFormat::parse(&format)
                    .with_context(|| format!("Invalid format: {}", format))?;

                // Determine source mode
                let source_mode = SnapshotExporter::<jit::JsonFileStorage>::determine_source_mode(
                    at.as_deref(),
                    working_tree,
                    committed_only,
                )?;

                // TODO: Add validation unless --force
                if !force {
                    executor.validate_silent()?;
                }

                // Create exporter and export
                let exporter = SnapshotExporter::new(storage);
                let output_path = exporter.export(
                    &snapshot_scope,
                    &source_mode,
                    &snapshot_format,
                    out.as_deref().map(std::path::Path::new),
                )?;

                if json {
                    use jit::output::JsonOutput;
                    use serde_json::json;

                    let output_data = json!({
                        "path": output_path,
                        "format": format,
                        "scope": scope,
                    });
                    println!(
                        "{}",
                        JsonOutput::success(output_data, "graph export").to_json_string()?
                    );
                }
            }
        },
    }

    Ok(())
}
