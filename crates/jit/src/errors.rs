//! Actionable error formatting for improved user experience.
//!
//! This module provides utilities for creating error messages with:
//! - Clear error description
//! - Possible causes (diagnostics)
//! - Remediation steps (actionable fixes)
//!
//! Designed to help users understand what went wrong and how to fix it.

use std::fmt;

/// An error with diagnostic context and remediation steps.
///
/// This struct wraps an error message with additional context to help users
/// diagnose and fix the problem.
///
/// # Example
///
/// ```
/// use jit::errors::ActionableError;
///
/// let error = ActionableError::new("Lease abc123 not found")
///     .with_cause("The lease may have expired")
///     .with_cause("The lease ID may be incorrect")
///     .with_remedy("List all active leases: jit claim list --json | jq -r '.data.leases[].lease_id'")
///     .with_remedy("Check if the issue is still claimed: jit claim status --issue <issue-id>");
///
/// eprintln!("{}", error);
/// ```
#[derive(Debug, Clone)]
pub struct ActionableError {
    /// The main error message
    error: String,
    /// Possible causes (diagnostic hints)
    causes: Vec<String>,
    /// Remediation steps (how to fix)
    remediation: Vec<String>,
}

impl ActionableError {
    /// Create a new actionable error with the given message.
    pub fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            causes: Vec::new(),
            remediation: Vec::new(),
        }
    }

    /// Add a possible cause (diagnostic hint).
    ///
    /// This helps users understand why the error might have occurred.
    pub fn with_cause(mut self, cause: impl Into<String>) -> Self {
        self.causes.push(cause.into());
        self
    }

    /// Add a remediation step (actionable fix).
    ///
    /// This tells users what they can do to fix the problem.
    pub fn with_remedy(mut self, remedy: impl Into<String>) -> Self {
        self.remediation.push(remedy.into());
        self
    }

    /// Convert to a formatted error message suitable for display.
    pub fn to_error_message(&self) -> String {
        let mut msg = format!("Error: {}\n", self.error);

        if !self.causes.is_empty() {
            msg.push_str("\nPossible causes:\n");
            for cause in &self.causes {
                msg.push_str(&format!("  • {}\n", cause));
            }
        }

        if !self.remediation.is_empty() {
            msg.push_str("\nTo fix:\n");
            for remedy in &self.remediation {
                msg.push_str(&format!("  • {}\n", remedy));
            }
        }

        msg
    }
}

impl fmt::Display for ActionableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_error_message())
    }
}

impl std::error::Error for ActionableError {}

/// Helper to create lease not found errors with standard remediation.
pub fn lease_not_found(lease_id: &str) -> ActionableError {
    ActionableError::new(format!("Lease {} not found", lease_id))
        .with_cause("The lease may have expired")
        .with_cause("The lease ID may be incorrect")
        .with_cause("The lease may have been released or evicted")
        .with_remedy(
            "List all active leases: jit claim list --json | jq -r '.data.leases[].lease_id'",
        )
        .with_remedy("Check lease status: jit claim status --json")
}

/// Helper to create already claimed errors with standard remediation.
pub fn already_claimed(issue_id: &str, agent_id: &str, expires_info: &str) -> ActionableError {
    ActionableError::new(format!(
        "Issue {} already claimed by {} {}",
        issue_id, agent_id, expires_info
    ))
    .with_cause("Another agent is currently working on this issue")
    .with_cause("The previous agent may have crashed without releasing the lease")
    .with_cause("The issue may still be in progress")
    .with_remedy(format!(
        "Wait for the lease to expire or be released: jit claim status --issue {} --json",
        issue_id
    ))
    .with_remedy(format!(
        "Contact the agent owner to coordinate: {}",
        agent_id
    ))
    .with_remedy("If the agent crashed, force evict with: jit claim force-evict <lease-id> --reason \"<reason>\"")
}

/// Helper to create git repository detection errors with standard remediation.
pub fn not_in_git_repo() -> ActionableError {
    ActionableError::new("Not in a git repository")
        .with_cause("Current directory is not part of a git repository")
        .with_cause("Git is not installed or not in PATH")
        .with_remedy("Initialize a git repository: git init")
        .with_remedy("Change to a directory inside a git repository")
        .with_remedy("Verify git is installed: git --version")
}

/// Helper to create git command failure errors with standard remediation.
pub fn git_command_failed(command: &str, stderr: &str) -> ActionableError {
    ActionableError::new(format!("Git command failed: {}", command))
        .with_cause(format!("Git error: {}", stderr.trim()))
        .with_cause("Repository may be in an invalid state")
        .with_cause("Git configuration may be incorrect")
        .with_remedy("Check repository status: git status")
        .with_remedy("Verify git configuration: git config --list")
        .with_remedy(format!("Try running the command manually: {}", command))
}

/// Helper to create ownership errors with standard remediation.
pub fn not_owner(resource: &str, owner: &str, requester: &str) -> ActionableError {
    ActionableError::new(format!(
        "Cannot modify {}: owned by {}, not {}",
        resource, owner, requester
    ))
    .with_cause("You are not the owner of this resource")
    .with_cause("The wrong agent ID may be configured")
    .with_remedy("Check your agent configuration: jit config show-agent")
    .with_remedy(format!("Use the correct agent ID: {}", owner))
    .with_remedy("If you need to override, use force-evict (requires reason)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_actionable_error_formatting() {
        let error = ActionableError::new("Test error")
            .with_cause("First cause")
            .with_cause("Second cause")
            .with_remedy("First remedy")
            .with_remedy("Second remedy");

        let msg = error.to_error_message();

        assert!(msg.contains("Error: Test error"));
        assert!(msg.contains("Possible causes:"));
        assert!(msg.contains("• First cause"));
        assert!(msg.contains("• Second cause"));
        assert!(msg.contains("To fix:"));
        assert!(msg.contains("• First remedy"));
        assert!(msg.contains("• Second remedy"));
    }

    #[test]
    fn test_error_without_causes() {
        let error = ActionableError::new("Simple error").with_remedy("Just fix it");

        let msg = error.to_error_message();

        assert!(msg.contains("Error: Simple error"));
        assert!(!msg.contains("Possible causes:"));
        assert!(msg.contains("To fix:"));
        assert!(msg.contains("• Just fix it"));
    }

    #[test]
    fn test_error_without_remediation() {
        let error = ActionableError::new("Diagnostic only").with_cause("Something went wrong");

        let msg = error.to_error_message();

        assert!(msg.contains("Error: Diagnostic only"));
        assert!(msg.contains("Possible causes:"));
        assert!(msg.contains("• Something went wrong"));
        assert!(!msg.contains("To fix:"));
    }

    #[test]
    fn test_lease_not_found_helper() {
        let error = lease_not_found("abc123");
        let msg = error.to_error_message();

        assert!(msg.contains("Lease abc123 not found"));
        assert!(msg.contains("jit claim list"));
        assert!(msg.contains("jit claim status"));
    }

    #[test]
    fn test_already_claimed_helper() {
        let error = already_claimed("issue-1", "agent:worker-1", "until 2026-01-15");
        let msg = error.to_error_message();

        assert!(msg.contains("Issue issue-1 already claimed"));
        assert!(msg.contains("agent:worker-1"));
        assert!(msg.contains("jit claim status --issue"));
        assert!(msg.contains("force-evict"));
    }

    #[test]
    fn test_not_in_git_repo_helper() {
        let error = not_in_git_repo();
        let msg = error.to_error_message();

        assert!(msg.contains("Not in a git repository"));
        assert!(msg.contains("git init"));
        assert!(msg.contains("git --version"));
    }
}
