# Implementation Plan: Add --quiet Flag for Scripting

## Problem Statement

When using JIT in scripts or piping commands, unnecessary output causes issues:

**Current Problems**:
1. **Verbose compilation output**: `cargo run` shows compilation messages
2. **Broken pipe panics**: When piping to `head`, `jq`, etc., programs panic with "Broken pipe (os error 32)"
3. **Progress messages**: Some commands show progress that clutters script output
4. **No distinction**: Between human-friendly output and machine-consumable output

**Example**:
```bash
$ jit query all | head -1
thread 'main' panicked at library/std/src/io/stdio.rs:1165:9:
failed printing to stdout: Broken pipe (os error 32)
```

Users have to use `2>/dev/null` everywhere to suppress stderr, which hides real errors.

## Goal

Add `--quiet` / `-q` flag that:
1. Suppresses all non-essential output
2. Only outputs the requested data
3. Handles broken pipes gracefully (no panic)
4. Works well with `--json` for machine-readable output

## Current State

### Output Sources
- **Essential**: Requested data (issue list, issue details, etc.)
- **Non-essential**: 
  - Success messages ("Added dependency...", "Marked as done", etc.)
  - Warnings (validation issues, deprecation notices)
  - Progress indicators
  - Decorative formatting (headers, separators)

### No Quiet Mode
- All commands output everything
- No way to suppress confirmations
- Scripts get cluttered output

## Task Breakdown

### 1. Add Global --quiet Flag

**File**: `crates/jit/src/cli.rs`

```rust
#[derive(Parser)]
#[command(name = "jit")]
pub struct Cli {
    /// Suppress non-essential output (for scripting)
    #[arg(short, long, global = true)]
    pub quiet: bool,
    
    /// Export command schema in JSON format
    #[arg(long)]
    pub schema: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}
```

### 2. Create Output Helper Module

**File**: `crates/jit/src/output.rs` (extend existing)

```rust
/// Context for controlling output verbosity
pub struct OutputContext {
    quiet: bool,
    json: bool,
}

impl OutputContext {
    pub fn new(quiet: bool, json: bool) -> Self {
        Self { quiet, json }
    }
    
    /// Print essential output (always shown unless --json)
    pub fn print_data(&self, msg: impl Display) {
        if !self.json {
            println!("{}", msg);
        }
    }
    
    /// Print informational message (suppressed by --quiet)
    pub fn print_info(&self, msg: impl Display) {
        if !self.quiet && !self.json {
            println!("{}", msg);
        }
    }
    
    /// Print success message (suppressed by --quiet)
    pub fn print_success(&self, msg: impl Display) {
        if !self.quiet && !self.json {
            println!("{}", msg);
        }
    }
    
    /// Print warning (suppressed by --quiet)
    pub fn print_warning(&self, msg: impl Display) {
        if !self.quiet && !self.json {
            eprintln!("Warning: {}", msg);
        }
    }
    
    /// Print error (always shown to stderr)
    pub fn print_error(&self, msg: impl Display) {
        eprintln!("Error: {}", msg);
    }
}
```

### 3. Update Commands to Use OutputContext

**Pattern to apply**:

```rust
// OLD - direct println
println!("Added dependency: {} depends on {}", from_id, to_id);

// NEW - use output context
output.print_success(format!("Added dependency: {} depends on {}", from_id, to_id));
```

**Commands to update** (in `crates/jit/src/main.rs` and command modules):

All commands that print:
- Success messages
- Confirmation messages
- Informational output
- Warnings

Keep essential data output in quiet mode:
- `issue list` - still shows issues
- `issue show` - still shows issue details
- `status` - still shows status summary

### 4. Handle Broken Pipe Gracefully

**File**: `crates/jit/src/main.rs`

```rust
use std::io::{self, Write};

fn main() -> ExitCode {
    // Install custom panic hook to handle broken pipe gracefully
    std::panic::set_hook(Box::new(|panic_info| {
        if let Some(msg) = panic_info.payload().downcast_ref::<&str>() {
            if msg.contains("Broken pipe") {
                // Silently exit on broken pipe (expected when piping to head, etc.)
                std::process::exit(0);
            }
        }
        // Otherwise use default panic handler
        eprintln!("Fatal error: {:?}", panic_info);
        std::process::exit(1);
    }));
    
    // Alternatively: ignore SIGPIPE
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    
    // Rest of main...
}
```

Or use result-based writing:

```rust
fn safe_println(msg: &str) {
    if let Err(e) = writeln!(io::stdout(), "{}", msg) {
        if e.kind() == io::ErrorKind::BrokenPipe {
            // Silently exit
            std::process::exit(0);
        }
        eprintln!("Write error: {}", e);
    }
}
```

### 5. Update Specific Command Behaviors

#### Issue Commands
```rust
// issue create
if !quiet {
    println!("Created issue: {}", issue.id);
}
println!("{}", issue.id); // Always output ID for scripting

// issue list
// Always output list, but suppress headers in quiet mode
if !quiet {
    println!("Issues:");
}
for issue in issues {
    println!("{}", format_issue(&issue, quiet));
}

// issue update
if !quiet {
    println!("Updated issue {}", id);
}
if json {
    print_json(&issue);
} else if quiet {
    // Just output the ID
    println!("{}", issue.id);
}
```

#### Dependency Commands
```rust
// dep add
if !quiet {
    println!("Added dependency: {} depends on {}", from_id, to_id);
}
// In quiet mode, output nothing (success indicated by exit code)

// dep rm
if !quiet {
    println!("Removed dependency: {} no longer depends on {}", from_id, to_id);
}
```

#### Status Command
```rust
// status
if json {
    print_json(&status);
} else if quiet {
    // Minimal output: just counts
    println!("{} {} {} {} {}", 
        status.open, status.ready, status.in_progress, status.done, status.blocked);
} else {
    // Full formatted output
    print_formatted_status(&status);
}
```

### 6. Add Tests

**File**: `crates/jit/tests/quiet_mode_tests.rs`

```rust
#[test]
fn test_quiet_mode_suppresses_success_messages() {
    let h = TestHarness::new();
    let issue = h.create_issue("Test");
    
    // Normal mode
    let output = h.run_command(&["issue", "update", &issue, "--state", "done"]);
    assert!(output.contains("Updated issue"));
    
    // Quiet mode
    let output = h.run_command(&["issue", "update", &issue, "--state", "done", "--quiet"]);
    assert!(!output.contains("Updated issue"));
}

#[test]
fn test_quiet_mode_preserves_essential_output() {
    let h = TestHarness::new();
    h.create_issue("Test 1");
    h.create_issue("Test 2");
    
    // Quiet mode should still output the list
    let output = h.run_command(&["issue", "list", "--quiet"]);
    assert!(output.contains("Test 1"));
    assert!(output.contains("Test 2"));
}

#[test]
fn test_quiet_with_json_outputs_only_json() {
    let h = TestHarness::new();
    let issue = h.create_issue("Test");
    
    let output = h.run_command(&["issue", "show", &issue, "--quiet", "--json"]);
    
    // Should be valid JSON with no extra text
    let json: serde_json::Value = serde_json::from_str(&output)
        .expect("Output should be pure JSON");
    assert_eq!(json["data"]["title"], "Test");
}

#[test]
fn test_broken_pipe_handled_gracefully() {
    let h = TestHarness::new();
    for i in 0..100 {
        h.create_issue(&format!("Issue {}", i));
    }
    
    // Pipe to head -1 (will close pipe after first line)
    let status = h.run_command_piped(&["issue", "list"], &["head", "-1"]);
    
    // Should exit cleanly (code 0), not panic
    assert_eq!(status.code(), Some(0));
}

#[test]
fn test_quiet_suppresses_warnings() {
    let h = TestHarness::new();
    
    // Create scenario that triggers warning
    let issue = h.create_issue_orphaned("Task");
    
    // Normal mode shows warning
    let output = h.run_command(&["issue", "show", &issue]);
    assert!(output.contains("Warning"));
    
    // Quiet mode suppresses it
    let output = h.run_command(&["issue", "show", &issue, "--quiet"]);
    assert!(!output.contains("Warning"));
}
```

### 7. Update Documentation

**README.md**:
```markdown
## Scripting and Automation

### Quiet Mode

Use `--quiet` / `-q` flag to suppress non-essential output:

```bash
# Create issue and capture ID
ISSUE_ID=$(jit issue create --title "Bug fix" --quiet)

# Update without confirmation message
jit issue update $ISSUE_ID --state done --quiet

# Pipe to other commands without noise
jit query all --quiet | grep "Bug"
```

### JSON Mode

Combine `--quiet` with `--json` for pure JSON output:

```bash
jit query all --quiet --json | jq '.data[0].id'
```
```

**EXAMPLE.md** - Add scripting examples section

## Implementation Approach

1. **Add --quiet flag** to CLI (15 min)
2. **Create OutputContext** helper (30 min)
3. **Update 10 commands** as proof of concept (2 hours)
4. **Handle broken pipe** gracefully (1 hour)
5. **Systematically update** all commands (4 hours)
6. **Add tests** (2 hours)
7. **Update documentation** (1 hour)
8. **Full test suite** + verify no regressions (1 hour)

**Total: 11-12 hours**

## Success Criteria

✅ `--quiet` flag globally available on all commands  
✅ Suppresses success messages, warnings, decorative output  
✅ Preserves essential data output  
✅ Broken pipe handled gracefully (no panic)  
✅ Works well with `--json` for pure JSON output  
✅ All tests pass  
✅ Documentation updated with scripting examples  
✅ No breaking changes to normal (non-quiet) output  

## Benefits

- **Better scripting**: Clean output for pipes and captures
- **No broken pipe panics**: Graceful handling
- **Machine-friendly**: Combine with --json for automation
- **User choice**: Verbose by default, quiet on demand
- **Professional**: Matches git, cargo, and other CLI tools

## Edge Cases

1. **Errors**: Always shown (to stderr) even in quiet mode
2. **JSON mode**: Quiet is automatic (only JSON output)
3. **Interactive prompts**: Disabled in quiet mode (fail instead)
4. **Progress bars**: Suppressed in quiet mode
5. **Exit codes**: Still indicate success/failure

## Dependencies

No new dependencies required.

## Related Issues

- Improves scripting and automation workflows
- Complements JSON output standardization
- Makes MCP integration cleaner
