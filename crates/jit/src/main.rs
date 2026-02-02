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
    ClaimCommands, Cli, Commands, DepCommands, DocCommands, EventCommands, GateCommands,
    GraphCommands, IssueCommands, RegistryCommands,
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
                        use jit::domain::MinimalIssue;
                        use jit::output::{GraphShowResponse, JsonOutput};

                        let minimal_issues: Vec<MinimalIssue> =
                            issues.iter().map(MinimalIssue::from).collect();
                        let response = GraphShowResponse {
                            issue_id: issue_id.clone(),
                            dependencies: minimal_issues,
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
            GraphCommands::Deps {
                id,
                transitive,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                let issues = executor.show_dependencies(&id, transitive)?;
                if json {
                    use jit::domain::MinimalIssue;
                    use jit::output::{GraphDepsResponse, JsonOutput};

                    let minimal_issues: Vec<MinimalIssue> =
                        issues.iter().map(MinimalIssue::from).collect();
                    let response = GraphDepsResponse {
                        issue_id: id.clone(),
                        dependencies: minimal_issues,
                        count: issues.len(),
                        transitive,
                        truncated: false,
                    };
                    let output = JsonOutput::success(response, "graph deps");
                    println!("{}", output.to_json_string()?);
                } else {
                    if transitive {
                        let _ = output_ctx
                            .print_info(format!("All dependencies of {} (transitive):", id));
                    } else {
                        let _ =
                            output_ctx.print_info(format!("Dependencies of {} (immediate):", id));
                    }
                    if issues.is_empty() {
                        println!("  (none)");
                    } else {
                        for issue in issues {
                            println!("  {} | {}", issue.id, issue.title);
                        }
                    }
                }
            }
            GraphCommands::Downstream { id, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let issues = executor.show_downstream(&id)?;
                if json {
                    use jit::domain::MinimalIssue;
                    use jit::output::{GraphDownstreamResponse, JsonOutput};

                    let minimal_issues: Vec<MinimalIssue> =
                        issues.iter().map(MinimalIssue::from).collect();
                    let response = GraphDownstreamResponse {
                        issue_id: id.clone(),
                        dependents: minimal_issues,
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
                    use jit::domain::MinimalIssue;
                    use jit::output::{GraphRootsResponse, JsonOutput};

                    let minimal_issues: Vec<MinimalIssue> =
                        issues.iter().map(MinimalIssue::from).collect();
                    let response = GraphRootsResponse {
                        roots: minimal_issues,
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
            jit::cli::QueryCommands::All {
                state,
                assignee,
                priority,
                label,
                full,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                let state_filter = state.as_ref().map(|s| parse_state(s)).transpose()?;
                let priority_filter = priority.as_ref().map(|p| parse_priority(p)).transpose()?;
                let issues = executor.query_all(
                    state_filter,
                    assignee.as_deref(),
                    priority_filter,
                    label.as_deref(),
                )?;

                if json {
                    use jit::domain::MinimalIssue;
                    use jit::output::JsonOutput;
                    use serde_json::json;

                    let output = if full {
                        JsonOutput::success(
                            json!({
                                "count": issues.len(),
                                "issues": issues,
                                "filters": {
                                    "state": state,
                                    "assignee": assignee,
                                    "priority": priority,
                                    "label": label,
                                }
                            }),
                            "query all",
                        )
                    } else {
                        let minimal: Vec<MinimalIssue> =
                            issues.iter().map(MinimalIssue::from).collect();
                        JsonOutput::success(
                            json!({
                                "count": minimal.len(),
                                "issues": minimal,
                                "filters": {
                                    "state": state,
                                    "assignee": assignee,
                                    "priority": priority,
                                    "label": label,
                                }
                            }),
                            "query all",
                        )
                    };
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info("All issues (filtered):");
                    for issue in &issues {
                        println!(
                            "  {} | {} | {:?} | {:?}",
                            issue.id, issue.title, issue.state, issue.priority
                        );
                    }
                    let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                }
            }
            jit::cli::QueryCommands::Available {
                priority,
                label,
                full,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                let priority_filter = priority.as_ref().map(|p| parse_priority(p)).transpose()?;
                let issues = executor.query_available(priority_filter, label.as_deref())?;

                if json {
                    use jit::domain::MinimalIssue;
                    use jit::output::JsonOutput;
                    use serde_json::json;

                    let output = if full {
                        JsonOutput::success(
                            json!({
                                "count": issues.len(),
                                "issues": issues,
                            }),
                            "query available",
                        )
                    } else {
                        let minimal: Vec<MinimalIssue> =
                            issues.iter().map(MinimalIssue::from).collect();
                        JsonOutput::success(
                            json!({
                                "count": minimal.len(),
                                "issues": minimal,
                            }),
                            "query available",
                        )
                    };
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info("Available issues (unassigned, unblocked):");
                    for issue in &issues {
                        println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                    }
                    let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                }
            }
            jit::cli::QueryCommands::Blocked {
                priority,
                label,
                full,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                let priority_filter = priority.as_ref().map(|p| parse_priority(p)).transpose()?;
                let blocked = executor.query_blocked_filtered(priority_filter, label.as_deref())?;

                if json {
                    use jit::domain::MinimalIssue;
                    use jit::output::JsonOutput;
                    use serde_json::json;

                    let output = if full {
                        use jit::output::{BlockedIssue, BlockedReason, BlockedReasonType};
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
                                    issue: MinimalIssue::from(issue),
                                    blocked_reasons,
                                }
                            })
                            .collect();

                        JsonOutput::success(
                            json!({
                                "count": blocked_issues.len(),
                                "issues": blocked_issues,
                            }),
                            "query blocked",
                        )
                    } else {
                        use jit::domain::MinimalBlockedIssue;
                        let minimal: Vec<MinimalBlockedIssue> = blocked
                            .iter()
                            .map(|(issue, reasons)| MinimalBlockedIssue {
                                id: issue.id.clone(),
                                title: issue.title.clone(),
                                state: issue.state,
                                priority: issue.priority,
                                blocked_reasons: reasons.clone(),
                            })
                            .collect();

                        JsonOutput::success(
                            json!({
                                "count": minimal.len(),
                                "issues": minimal,
                            }),
                            "query blocked",
                        )
                    };
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
            jit::cli::QueryCommands::Strategic {
                priority,
                label,
                full,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                let priority_filter = priority.as_ref().map(|p| parse_priority(p)).transpose()?;
                let issues =
                    executor.query_strategic_filtered(priority_filter, label.as_deref())?;

                if json {
                    use jit::domain::MinimalIssue;
                    use jit::output::JsonOutput;
                    use serde_json::json;

                    let output = if full {
                        JsonOutput::success(
                            json!({
                                "count": issues.len(),
                                "issues": issues,
                            }),
                            "query strategic",
                        )
                    } else {
                        let minimal: Vec<MinimalIssue> =
                            issues.iter().map(MinimalIssue::from).collect();
                        JsonOutput::success(
                            json!({
                                "count": minimal.len(),
                                "issues": minimal,
                            }),
                            "query strategic",
                        )
                    };
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info("Strategic issues:");
                    for issue in &issues {
                        println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                    }
                    let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                }
            }
            jit::cli::QueryCommands::Closed {
                priority,
                label,
                full,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                let priority_filter = priority.as_ref().map(|p| parse_priority(p)).transpose()?;
                let issues = executor.query_closed_filtered(priority_filter, label.as_deref())?;

                if json {
                    use jit::domain::MinimalIssue;
                    use jit::output::JsonOutput;
                    use serde_json::json;

                    let output = if full {
                        JsonOutput::success(
                            json!({
                                "count": issues.len(),
                                "issues": issues,
                            }),
                            "query closed",
                        )
                    } else {
                        let minimal: Vec<MinimalIssue> =
                            issues.iter().map(MinimalIssue::from).collect();
                        JsonOutput::success(
                            json!({
                                "count": minimal.len(),
                                "issues": minimal,
                            }),
                            "query closed",
                        )
                    };
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
            jit::cli::ConfigCommands::Show { json } => {
                use jit::config::ConfigLoader;
                use jit::output::JsonOutput;
                use serde_json::json;

                // Build effective config from all sources
                let mut loader = ConfigLoader::new();

                // Try to load system config
                let system_path = std::path::Path::new("/etc/jit");
                if system_path.exists() {
                    loader = loader.with_system_config(system_path)?;
                }

                // Try to load user config
                if let Some(home) = dirs::home_dir() {
                    let user_path = home.join(".config/jit");
                    if user_path.exists() {
                        loader = loader.with_user_config(&user_path)?;
                    }
                }

                // Load repo config
                loader = loader.with_repo_config(&jit_dir)?;

                let config = loader.build();

                if json {
                    let output = json!({
                        "worktree": {
                            "mode": format!("{:?}", config.worktree_mode().unwrap_or(jit::config::WorktreeMode::Auto)).to_lowercase(),
                            "enforce_leases": format!("{:?}", config.enforcement_mode().unwrap_or(jit::config::EnforcementMode::Strict)).to_lowercase(),
                        },
                        "coordination": {
                            "default_ttl_secs": config.coordination().default_ttl_secs(),
                            "heartbeat_interval_secs": config.coordination().heartbeat_interval_secs(),
                            "lease_renewal_threshold_pct": config.coordination().lease_renewal_threshold_pct(),
                            "stale_threshold_secs": config.coordination().stale_threshold_secs(),
                            "max_indefinite_leases_per_agent": config.coordination().max_indefinite_leases_per_agent(),
                            "max_indefinite_leases_per_repo": config.coordination().max_indefinite_leases_per_repo(),
                            "auto_renew_leases": config.coordination().auto_renew_leases(),
                        },
                        "global_operations": {
                            "require_main_history": config.global_operations().require_main_history(),
                            "allowed_branches": config.global_operations().allowed_branches(),
                        },
                        "locks": {
                            "max_age_secs": config.locks().max_age_secs(),
                            "enable_metadata": config.locks().enable_metadata(),
                        },
                        "events": {
                            "enable_sequences": config.events().enable_sequences(),
                            "use_unified_envelope": config.events().use_unified_envelope(),
                        },
                    });
                    println!(
                        "{}",
                        JsonOutput::success(output, "config show").to_json_string()?
                    );
                } else {
                    println!("Effective Configuration:");
                    println!();
                    println!("[worktree]");
                    println!(
                        "  mode = {:?}",
                        config
                            .worktree_mode()
                            .unwrap_or(jit::config::WorktreeMode::Auto)
                    );
                    println!(
                        "  enforce_leases = {:?}",
                        config
                            .enforcement_mode()
                            .unwrap_or(jit::config::EnforcementMode::Strict)
                    );
                    println!();
                    println!("[coordination]");
                    println!(
                        "  default_ttl_secs = {}",
                        config.coordination().default_ttl_secs()
                    );
                    println!(
                        "  heartbeat_interval_secs = {}",
                        config.coordination().heartbeat_interval_secs()
                    );
                    println!(
                        "  lease_renewal_threshold_pct = {}",
                        config.coordination().lease_renewal_threshold_pct()
                    );
                    println!(
                        "  stale_threshold_secs = {}",
                        config.coordination().stale_threshold_secs()
                    );
                    println!(
                        "  max_indefinite_leases_per_agent = {}",
                        config.coordination().max_indefinite_leases_per_agent()
                    );
                    println!(
                        "  max_indefinite_leases_per_repo = {}",
                        config.coordination().max_indefinite_leases_per_repo()
                    );
                    println!(
                        "  auto_renew_leases = {}",
                        config.coordination().auto_renew_leases()
                    );
                    println!();
                    println!("[global_operations]");
                    println!(
                        "  require_main_history = {}",
                        config.global_operations().require_main_history()
                    );
                    println!(
                        "  allowed_branches = {:?}",
                        config.global_operations().allowed_branches()
                    );
                    println!();
                    println!("[locks]");
                    println!("  max_age_secs = {}", config.locks().max_age_secs());
                    println!("  enable_metadata = {}", config.locks().enable_metadata());
                    println!();
                    println!("[events]");
                    println!(
                        "  enable_sequences = {}",
                        config.events().enable_sequences()
                    );
                    println!(
                        "  use_unified_envelope = {}",
                        config.events().use_unified_envelope()
                    );
                }
            }
            jit::cli::ConfigCommands::Get { key, json } => {
                use jit::config::ConfigLoader;
                use jit::output::JsonOutput;
                use serde_json::json;

                // Build effective config
                let mut loader = ConfigLoader::new();
                let system_path = std::path::Path::new("/etc/jit");
                if system_path.exists() {
                    loader = loader.with_system_config(system_path)?;
                }
                if let Some(home) = dirs::home_dir() {
                    let user_path = home.join(".config/jit");
                    if user_path.exists() {
                        loader = loader.with_user_config(&user_path)?;
                    }
                }
                loader = loader.with_repo_config(&jit_dir)?;
                let config = loader.build();

                // Parse key and get value
                let value: Option<serde_json::Value> = match key.as_str() {
                    "worktree.mode" => Some(json!(format!(
                        "{:?}",
                        config
                            .worktree_mode()
                            .unwrap_or(jit::config::WorktreeMode::Auto)
                    )
                    .to_lowercase())),
                    "worktree.enforce_leases" => Some(json!(format!(
                        "{:?}",
                        config
                            .enforcement_mode()
                            .unwrap_or(jit::config::EnforcementMode::Strict)
                    )
                    .to_lowercase())),
                    "coordination.default_ttl_secs" => {
                        Some(json!(config.coordination().default_ttl_secs()))
                    }
                    "coordination.heartbeat_interval_secs" => {
                        Some(json!(config.coordination().heartbeat_interval_secs()))
                    }
                    "coordination.lease_renewal_threshold_pct" => {
                        Some(json!(config.coordination().lease_renewal_threshold_pct()))
                    }
                    "coordination.stale_threshold_secs" => {
                        Some(json!(config.coordination().stale_threshold_secs()))
                    }
                    "coordination.max_indefinite_leases_per_agent" => Some(json!(config
                        .coordination()
                        .max_indefinite_leases_per_agent())),
                    "coordination.max_indefinite_leases_per_repo" => Some(json!(config
                        .coordination()
                        .max_indefinite_leases_per_repo())),
                    "coordination.auto_renew_leases" => {
                        Some(json!(config.coordination().auto_renew_leases()))
                    }
                    "global_operations.require_main_history" => {
                        Some(json!(config.global_operations().require_main_history()))
                    }
                    "global_operations.allowed_branches" => {
                        Some(json!(config.global_operations().allowed_branches()))
                    }
                    "locks.max_age_secs" => Some(json!(config.locks().max_age_secs())),
                    "locks.enable_metadata" => Some(json!(config.locks().enable_metadata())),
                    "events.enable_sequences" => Some(json!(config.events().enable_sequences())),
                    "events.use_unified_envelope" => {
                        Some(json!(config.events().use_unified_envelope()))
                    }
                    _ => None,
                };

                match value {
                    Some(v) => {
                        if json {
                            println!(
                                "{}",
                                JsonOutput::success(json!({"key": key, "value": v}), "config get")
                                    .to_json_string()?
                            );
                        } else {
                            println!("{}", v);
                        }
                    }
                    None => {
                        anyhow::bail!(
                            "Unknown config key: {}. Use 'jit config show' to see available keys.",
                            key
                        );
                    }
                }
            }
            jit::cli::ConfigCommands::Set {
                key,
                value,
                global,
                json,
            } => {
                use jit::output::JsonOutput;
                use serde_json::json;
                use std::fs;

                // Determine target config file
                let config_path = if global {
                    let home = dirs::home_dir()
                        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
                    let config_dir = home.join(".config/jit");
                    fs::create_dir_all(&config_dir)?;
                    config_dir.join("config.toml")
                } else {
                    jit_dir.join("config.toml")
                };

                // Load existing config or create empty
                let mut doc = if config_path.exists() {
                    let content = fs::read_to_string(&config_path)?;
                    content
                        .parse::<toml_edit::DocumentMut>()
                        .map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))?
                } else {
                    toml_edit::DocumentMut::new()
                };

                // Parse key into section.field
                let parts: Vec<&str> = key.split('.').collect();
                if parts.len() != 2 {
                    anyhow::bail!("Config key must be in format 'section.field' (e.g., coordination.default_ttl_secs)");
                }
                let section = parts[0];
                let field = parts[1];

                // Ensure section exists
                if doc.get(section).is_none() {
                    doc[section] = toml_edit::Item::Table(toml_edit::Table::new());
                }

                // Parse and set value based on expected type
                let parsed_value: toml_edit::Item = match key.as_str() {
                    k if k.ends_with("_secs") || k.ends_with("_pct") || k.contains("max_") => {
                        let num: i64 = value
                            .parse()
                            .map_err(|_| anyhow::anyhow!("Expected numeric value for {}", key))?;
                        toml_edit::value(num)
                    }
                    k if k.contains("enable_") || k.contains("require_") || k.contains("auto_") => {
                        let b: bool = value.parse().map_err(|_| {
                            anyhow::anyhow!("Expected boolean (true/false) for {}", key)
                        })?;
                        toml_edit::value(b)
                    }
                    _ => toml_edit::value(&value),
                };

                doc[section][field] = parsed_value;

                // Write back
                fs::write(&config_path, doc.to_string())?;

                if json {
                    println!(
                        "{}",
                        JsonOutput::success(
                            json!({
                                "key": key,
                                "value": value,
                                "file": config_path.display().to_string(),
                                "scope": if global { "user" } else { "repo" }
                            }),
                            "config set"
                        )
                        .to_json_string()?
                    );
                } else {
                    println!("Set {} = {} in {}", key, value, config_path.display());
                }
            }
            jit::cli::ConfigCommands::Validate { json } => {
                use jit::config::{ConfigLoader, JitConfig};
                use jit::output::JsonOutput;
                use serde_json::json;

                #[derive(Default)]
                struct ValidationResult {
                    errors: Vec<String>,
                    warnings: Vec<String>,
                }

                let mut result = ValidationResult::default();

                // Check repo config
                let repo_config_path = jit_dir.join("config.toml");
                if repo_config_path.exists() {
                    match JitConfig::load(&jit_dir) {
                        Ok(cfg) => {
                            // Check for invalid values
                            if let Some(ref wt) = cfg.worktree {
                                if let Err(e) = wt.worktree_mode() {
                                    result.errors.push(format!("repo config: {}", e));
                                }
                                if let Err(e) = wt.enforcement_mode() {
                                    result.errors.push(format!("repo config: {}", e));
                                }
                            }
                        }
                        Err(e) => {
                            result.errors.push(format!("repo config: {}", e));
                        }
                    }
                }

                // Check user config
                if let Some(home) = dirs::home_dir() {
                    let user_config_path = home.join(".config/jit/config.toml");
                    if user_config_path.exists() {
                        let user_dir = home.join(".config/jit");
                        match JitConfig::load(&user_dir) {
                            Ok(cfg) => {
                                if let Some(ref wt) = cfg.worktree {
                                    if let Err(e) = wt.worktree_mode() {
                                        result.errors.push(format!("user config: {}", e));
                                    }
                                    if let Err(e) = wt.enforcement_mode() {
                                        result.errors.push(format!("user config: {}", e));
                                    }
                                }
                            }
                            Err(e) => {
                                result.errors.push(format!("user config: {}", e));
                            }
                        }
                    }
                }

                // Check env vars
                if let Ok(val) = std::env::var("JIT_WORKTREE_MODE") {
                    if !["auto", "on", "off"].contains(&val.to_lowercase().as_str()) {
                        result.errors.push(format!(
                            "JIT_WORKTREE_MODE='{}' is invalid. Use: auto, on, off",
                            val
                        ));
                    }
                }
                if let Ok(val) = std::env::var("JIT_ENFORCE_LEASES") {
                    if !["strict", "warn", "off"].contains(&val.to_lowercase().as_str()) {
                        result.errors.push(format!(
                            "JIT_ENFORCE_LEASES='{}' is invalid. Use: strict, warn, off",
                            val
                        ));
                    }
                }

                // Try to build effective config to catch merge issues
                let loader = ConfigLoader::new();
                let _ = loader.with_repo_config(&jit_dir);

                let has_errors = !result.errors.is_empty();
                let has_warnings = !result.warnings.is_empty();

                if json {
                    let output = json!({
                        "valid": !has_errors,
                        "errors": result.errors,
                        "warnings": result.warnings,
                    });
                    println!(
                        "{}",
                        JsonOutput::success(output, "config validate").to_json_string()?
                    );
                } else if result.errors.is_empty() && result.warnings.is_empty() {
                    println!("✓ Configuration is valid");
                } else {
                    if !result.errors.is_empty() {
                        println!("Errors:");
                        for err in &result.errors {
                            println!("  ✗ {}", err);
                        }
                    }
                    if !result.warnings.is_empty() {
                        println!("Warnings:");
                        for warn in &result.warnings {
                            println!("  ⚠ {}", warn);
                        }
                    }
                }

                // Exit with appropriate code
                if has_errors {
                    std::process::exit(1);
                } else if has_warnings {
                    std::process::exit(2);
                }
            }
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
        Commands::Hooks(hooks_cmd) => match hooks_cmd {
            jit::cli::HooksCommands::Install { json } => {
                use jit::commands::hooks::install_hooks;

                match install_hooks(None) {
                    Ok(result) => {
                        if json {
                            let output = jit::output::JsonOutput::success(
                                serde_json::json!({
                                    "hooks_dir": result.hooks_dir,
                                    "installed": result.installed,
                                    "skipped": result.skipped,
                                }),
                                "hooks install",
                            );
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("Installed hooks to: {}", result.hooks_dir);
                            if !result.installed.is_empty() {
                                println!("\nInstalled:");
                                for hook in &result.installed {
                                    println!("  ✓ {}", hook);
                                }
                            }
                            if !result.skipped.is_empty() {
                                println!("\nSkipped (already exist):");
                                for hook in &result.skipped {
                                    println!("  - {}", hook);
                                }
                            }
                            println!("\nHooks are now active. Configure enforcement in .jit/config.toml:");
                            println!("  [worktree]");
                            println!("  enforce_leases = \"strict\"");
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error = jit::output::JsonError::new(
                                "HOOKS_INSTALL_ERROR",
                                e.to_string(),
                                "hooks install",
                            );
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
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
        Commands::Validate {
            json,
            fix,
            dry_run,
            divergence,
            leases,
        } => {
            // Validate dry_run requires fix
            if dry_run && !fix {
                return Err(anyhow!("--dry-run requires --fix to be specified"));
            }

            // Handle specific validations if requested
            if divergence || leases {
                let mut validation_results = Vec::new();

                if divergence {
                    match executor.validate_divergence() {
                        Ok(()) => {
                            if !json {
                                println!("✓ Branch is up-to-date with origin/main");
                            }
                            validation_results.push(("divergence", true, String::new()));
                        }
                        Err(e) => {
                            if json {
                                validation_results.push(("divergence", false, e.to_string()));
                            } else {
                                eprintln!("❌ Divergence validation failed:\n{}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                }

                if leases {
                    match executor.validate_leases() {
                        Ok(invalid_leases) => {
                            if invalid_leases.is_empty() {
                                if !json {
                                    println!("✓ All active leases are valid");
                                }
                                validation_results.push(("leases", true, String::new()));
                            } else {
                                let message = format!(
                                    "Found {} invalid lease(s):\n{}",
                                    invalid_leases.len(),
                                    invalid_leases.join("\n\n")
                                );
                                if json {
                                    validation_results.push(("leases", false, message.clone()));
                                } else {
                                    eprintln!("❌ Lease validation failed:\n{}", message);
                                    std::process::exit(1);
                                }
                            }
                        }
                        Err(e) => {
                            if json {
                                validation_results.push(("leases", false, format!("Error: {}", e)));
                            } else {
                                eprintln!("❌ Lease validation error: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                }

                if json {
                    use jit::output::JsonOutput;
                    use serde_json::json;

                    let all_valid = validation_results.iter().all(|(_, valid, _)| *valid);
                    let results_json: Vec<_> = validation_results
                        .iter()
                        .map(|(name, valid, message)| {
                            json!({
                                "validation": name,
                                "valid": valid,
                                "message": message
                            })
                        })
                        .collect();

                    let output = JsonOutput::success(
                        json!({
                            "valid": all_valid,
                            "validations": results_json
                        }),
                        "validate",
                    );
                    println!("{}", output.to_json_string()?);

                    if !all_valid {
                        std::process::exit(1);
                    }
                }

                return Ok(());
            }

            // Standard repository validation (existing code)
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
        Commands::Recover { json } => {
            use jit::commands::claim::execute_recover;
            use jit::output::JsonOutput;
            use serde_json::json;

            match execute_recover(&storage) {
                Ok(report) => {
                    if json {
                        let output = JsonOutput::success(
                            json!({
                                "success": true,
                                "stale_locks_cleaned": report.stale_locks_cleaned,
                                "index_rebuilt": report.index_rebuilt,
                                "expired_leases_evicted": report.expired_leases_evicted,
                                "temp_files_removed": report.temp_files_removed,
                            }),
                            "recover",
                        );
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!("Recovery complete:");
                        println!("  • Stale locks cleaned: {}", report.stale_locks_cleaned);
                        println!("  • Index rebuilt: {}", report.index_rebuilt);
                        println!(
                            "  • Expired leases evicted: {}",
                            report.expired_leases_evicted
                        );
                        println!("  • Temp files removed: {}", report.temp_files_removed);
                    }
                }
                Err(e) => {
                    if json {
                        let output = jit::output::JsonError::new(
                            "recovery_failed",
                            e.to_string(),
                            "recover",
                        );
                        eprintln!("{}", serde_json::to_string(&output)?);
                        std::process::exit(1);
                    } else {
                        eprintln!("Recovery failed: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::Claim(claim_cmd) => match claim_cmd {
            ClaimCommands::Acquire {
                issue_id,
                ttl,
                agent_id,
                reason,
                json,
            } => {
                use jit::commands::claim::execute_claim_acquire;
                use jit::output::{JsonError, JsonOutput};

                match execute_claim_acquire(
                    &storage,
                    &issue_id,
                    ttl,
                    agent_id.as_deref(),
                    reason.as_deref(),
                ) {
                    Ok(lease_id) => {
                        if json {
                            let response = serde_json::json!({
                                "lease_id": lease_id,
                                "issue_id": issue_id,
                                "ttl_secs": ttl,
                                "message": format!("Acquired lease {} on issue {}", lease_id, issue_id),
                            });
                            let output = JsonOutput::success(response, "claim acquire");
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("✓ Acquired lease: {}", lease_id);
                            println!("  Issue: {}", issue_id);
                            println!("  TTL: {} seconds", ttl);
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error = JsonError::new(
                                "CLAIM_ACQUIRE_ERROR",
                                e.to_string(),
                                "claim acquire",
                            );
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            ClaimCommands::Release { lease_id, json } => {
                use jit::commands::claim::execute_claim_release;
                use jit::output::{JsonError, JsonOutput};

                match execute_claim_release(&lease_id) {
                    Ok(()) => {
                        if json {
                            let response = serde_json::json!({
                                "lease_id": lease_id,
                                "message": format!("Released lease {}", lease_id),
                            });
                            let output = JsonOutput::success(response, "claim release");
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("✓ Released lease: {}", lease_id);
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error = JsonError::new(
                                "CLAIM_RELEASE_ERROR",
                                e.to_string(),
                                "claim release",
                            );
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            ClaimCommands::Renew {
                lease_id,
                extension,
                json,
            } => {
                use jit::commands::claim::execute_claim_renew;
                use jit::output::{JsonError, JsonOutput};

                match execute_claim_renew::<jit::JsonFileStorage>(&lease_id, extension) {
                    Ok(renewed_lease) => {
                        if json {
                            let response = serde_json::json!({
                                "lease": renewed_lease,
                                "message": format!("Renewed lease {} by {} seconds", lease_id, extension),
                            });
                            let output = JsonOutput::success(response, "claim renew");
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("✓ Renewed lease: {}", lease_id);
                            println!("  Issue: {}", renewed_lease.issue_id);
                            println!("  Extended by: {} seconds", extension);
                            if let Some(expires_at) = renewed_lease.expires_at {
                                println!("  New expiry: {}", expires_at.to_rfc3339());
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error =
                                JsonError::new("CLAIM_RENEW_ERROR", e.to_string(), "claim renew");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            ClaimCommands::Heartbeat { lease_id, json } => {
                use jit::commands::claim::execute_claim_heartbeat;
                use jit::output::{JsonError, JsonOutput};

                match execute_claim_heartbeat(&lease_id) {
                    Ok(()) => {
                        if json {
                            let response = serde_json::json!({
                                "lease_id": lease_id,
                                "message": format!("Heartbeat sent for lease {}", lease_id),
                            });
                            let output = JsonOutput::success(response, "claim heartbeat");
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("✓ Heartbeat sent: {}", lease_id);
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error = JsonError::new(
                                "CLAIM_HEARTBEAT_ERROR",
                                e.to_string(),
                                "claim heartbeat",
                            );
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            ClaimCommands::Status { issue, agent, json } => {
                use jit::commands::claim::execute_claim_status;
                use jit::output::{JsonError, JsonOutput};

                match execute_claim_status::<jit::JsonFileStorage>(
                    issue.as_deref(),
                    agent.as_deref(),
                ) {
                    Ok(leases) => {
                        if json {
                            let response = serde_json::json!({
                                "leases": leases,
                                "count": leases.len(),
                            });
                            let output = JsonOutput::success(response, "claim status");
                            println!("{}", output.to_json_string()?);
                        } else if leases.is_empty() {
                            println!("No active leases found.");
                        } else {
                            use chrono::Utc;
                            println!("Active leases ({}):\n", leases.len());
                            for lease in &leases {
                                println!("Lease: {}", lease.lease_id);
                                println!("  Issue:    {}", lease.issue_id);
                                println!("  Agent:    {}", lease.agent_id);
                                println!("  Worktree: {}", lease.worktree_id);
                                if let Some(branch) = &lease.branch {
                                    println!("  Branch:   {}", branch);
                                }
                                println!("  Acquired: {}", lease.acquired_at);

                                if lease.ttl_secs > 0 {
                                    // Finite lease - show expiry and remaining time
                                    if let Some(expires_at) = lease.expires_at {
                                        let now = Utc::now();
                                        let remaining = expires_at.signed_duration_since(now);
                                        println!(
                                            "  Expires:  {} ({} seconds remaining)",
                                            expires_at,
                                            remaining.num_seconds().max(0)
                                        );
                                    }
                                } else {
                                    // Indefinite lease - show last beat and time since
                                    let now = Utc::now();
                                    let since_beat = now.signed_duration_since(lease.last_beat);
                                    println!("  TTL:      indefinite");
                                    println!(
                                        "  Last beat: {} ({} seconds ago)",
                                        lease.last_beat,
                                        since_beat.num_seconds()
                                    );

                                    // Show stale status
                                    if lease.stale {
                                        println!(
                                            "  ⚠️  STALE: Lease marked stale (no heartbeat for {} minutes)",
                                            since_beat.num_minutes()
                                        );
                                        println!(
                                            "     Use 'jit claim heartbeat {}' to refresh",
                                            lease.lease_id
                                        );
                                    }
                                }
                                println!();
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error =
                                JsonError::new("CLAIM_STATUS_ERROR", e.to_string(), "claim status");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            ClaimCommands::List { json } => {
                use jit::commands::claim::execute_claim_list;
                use jit::output::{JsonError, JsonOutput};

                match execute_claim_list() {
                    Ok(leases) => {
                        if json {
                            let response = serde_json::json!({
                                "leases": leases,
                                "count": leases.len(),
                            });
                            let output = JsonOutput::success(response, "claim list");
                            println!("{}", output.to_json_string()?);
                        } else if leases.is_empty() {
                            println!("No active leases.");
                        } else {
                            use chrono::Utc;
                            println!("All active leases ({}):\n", leases.len());
                            for lease in &leases {
                                println!("Lease: {}", lease.lease_id);
                                println!("  Issue:    {}", lease.issue_id);
                                println!("  Agent:    {}", lease.agent_id);
                                println!("  Worktree: {}", lease.worktree_id);
                                if let Some(branch) = &lease.branch {
                                    println!("  Branch:   {}", branch);
                                }
                                println!("  Acquired: {}", lease.acquired_at);

                                if lease.ttl_secs > 0 {
                                    // Finite lease
                                    if let Some(expires_at) = lease.expires_at {
                                        let now = Utc::now();
                                        let remaining = expires_at.signed_duration_since(now);
                                        println!(
                                            "  Expires:  {} ({} seconds remaining)",
                                            expires_at,
                                            remaining.num_seconds().max(0)
                                        );
                                    }
                                } else {
                                    // Indefinite lease
                                    let now = Utc::now();
                                    let since_beat = now.signed_duration_since(lease.last_beat);
                                    println!("  TTL:      indefinite");
                                    println!(
                                        "  Last beat: {} ({} seconds ago)",
                                        lease.last_beat,
                                        since_beat.num_seconds()
                                    );
                                }
                                println!();
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error =
                                JsonError::new("CLAIM_LIST_ERROR", e.to_string(), "claim list");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            ClaimCommands::ForceEvict {
                lease_id,
                reason,
                json,
            } => {
                use jit::commands::claim::execute_claim_force_evict;
                use jit::output::{JsonError, JsonOutput};

                match execute_claim_force_evict::<jit::JsonFileStorage>(&lease_id, &reason) {
                    Ok(()) => {
                        if json {
                            let response = serde_json::json!({
                                "lease_id": lease_id,
                                "reason": reason,
                                "message": format!("Force-evicted lease {}", lease_id),
                            });
                            let output = JsonOutput::success(response, "claim force-evict");
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("✓ Force-evicted lease: {}", lease_id);
                            println!("  Reason: {}", reason);
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error = JsonError::new(
                                "CLAIM_FORCE_EVICT_ERROR",
                                e.to_string(),
                                "claim force-evict",
                            );
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
        },
        Commands::Worktree(worktree_cmd) => match worktree_cmd {
            jit::cli::WorktreeCommands::Info { json } => {
                use jit::commands::worktree::execute_worktree_info;
                use jit::output::{JsonError, JsonOutput};

                match execute_worktree_info() {
                    Ok(info) => {
                        if json {
                            let response = serde_json::json!({
                                "worktree_id": info.worktree_id,
                                "branch": info.branch,
                                "root_path": info.root_path,
                                "is_main_worktree": info.is_main_worktree,
                                "common_dir": info.common_dir,
                            });
                            let output = JsonOutput::success(response, "worktree info");
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("Worktree Information:");
                            println!("  ID:         {}", info.worktree_id);
                            println!("  Branch:     {}", info.branch);
                            println!("  Root:       {}", info.root_path);
                            println!(
                                "  Type:       {}",
                                if info.is_main_worktree {
                                    "main worktree"
                                } else {
                                    "secondary worktree"
                                }
                            );
                            println!("  Common dir: {}", info.common_dir);
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error = JsonError::new(
                                "WORKTREE_INFO_ERROR",
                                e.to_string(),
                                "worktree info",
                            );
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            jit::cli::WorktreeCommands::List { json } => {
                use jit::commands::worktree::execute_worktree_list;
                use jit::output::{JsonError, JsonOutput};

                match execute_worktree_list() {
                    Ok(worktrees) => {
                        if json {
                            let response = serde_json::json!({
                                "worktrees": worktrees,
                            });
                            let output = JsonOutput::success(response, "worktree list");
                            println!("{}", output.to_json_string()?);
                        } else {
                            // Human-readable table format
                            println!(
                                "{:<16} {:<25} {:<50} {:>6}",
                                "WORKTREE ID", "BRANCH", "PATH", "CLAIMS"
                            );
                            println!("{}", "-".repeat(100));

                            for entry in worktrees {
                                println!(
                                    "{:<16} {:<25} {:<50} {:>6}",
                                    entry.worktree_id,
                                    entry.branch,
                                    entry.path,
                                    entry.active_claims
                                );
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error = JsonError::new(
                                "WORKTREE_LIST_ERROR",
                                e.to_string(),
                                "worktree list",
                            );
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
        },
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
