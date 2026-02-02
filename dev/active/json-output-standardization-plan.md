# Implementation Plan: Standardize JSON Output Format

## Problem Statement

CLI commands return JSON in inconsistent formats, making scripting and MCP integration difficult:

**Format 1** (some commands):
```json
{
  "id": "...",
  "title": "...",
  ...
}
```

**Format 2** (other commands):
```json
{
  "success": true,
  "data": {
    "id": "...",
    "title": "..."
  }
}
```

**Format 3** (errors - sometimes):
```json
{
  "error": "...",
  "code": "..."
}
```

This inconsistency forces users to handle multiple formats and makes client libraries complex.

## Current State

### Commands Using Wrapped Format (success/data)
- `jit issue create --json`
- `jit issue update --json`
- Most mutation commands

### Commands Using Direct Format
- `jit query all --json`
- `jit query * --json`
- Most read-only commands

### Error Handling
- Some commands use wrapped error format
- Others write to stderr and exit with codes
- Inconsistent between commands

## Goal

**Single, consistent JSON envelope format** for all commands with `--json` flag:

```json
{
  "success": true,
  "data": <actual-result>,
  "meta": {
    "timestamp": "2025-12-21T20:00:00Z",
    "version": "1.0.0",
    "command": "issue show"
  }
}
```

**Error format:**
```json
{
  "success": false,
  "error": {
    "code": "ISSUE_NOT_FOUND",
    "message": "Issue not found: abc123",
    "details": {}
  },
  "meta": {
    "timestamp": "2025-12-21T20:00:00Z",
    "version": "1.0.0",
    "command": "issue show"
  }
}
```

## Task Breakdown

### 1. Create Standard JSON Response Types

**File**: `crates/jit/src/output.rs` (already exists)

Add unified response types:

```rust
/// Standard JSON response envelope
#[derive(Debug, Serialize)]
pub struct JsonResponse<T> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonError>,
    pub meta: ResponseMeta,
}

#[derive(Debug, Serialize)]
pub struct JsonError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct ResponseMeta {
    pub timestamp: String,
    pub version: String,
    pub command: String,
}

impl<T: Serialize> JsonResponse<T> {
    pub fn success(data: T, command: &str) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            meta: ResponseMeta {
                timestamp: chrono::Utc::now().to_rfc3339(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                command: command.to_string(),
            },
        }
    }
    
    pub fn error(code: &str, message: &str, command: &str) -> JsonResponse<()> {
        JsonResponse {
            success: false,
            data: None,
            error: Some(JsonError {
                code: code.to_string(),
                message: message.to_string(),
                details: None,
            }),
            meta: ResponseMeta {
                timestamp: chrono::Utc::now().to_rfc3339(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                command: command.to_string(),
            },
        }
    }
}
```

### 2. Update All Commands to Use Standard Format

**Pattern to apply**:

```rust
// OLD - inconsistent formats
if json {
    println!("{}", serde_json::to_string_pretty(&issue)?);
}

// NEW - standard envelope
if json {
    let response = JsonResponse::success(issue, "issue show");
    println!("{}", serde_json::to_string_pretty(&response)?);
}
```

**Commands to update** (in `crates/jit/src/main.rs`):

#### Issue Commands
- `issue create` ✅ (already uses wrapper, verify format)
- `issue list` ❌ (direct format, needs wrapper)
- `issue show` ❌ (direct format)
- `issue update` ✅ (verify)
- `issue delete` (check)
- `issue search` ❌ (direct)
- `issue claim` (check)
- `issue assign` (check)
- `issue unassign` (check)
- `issue release` (check)
- `issue reject` (check)
- `issue breakdown` (check)

#### Query Commands
- `query state` ❌ (direct)
- `query priority` ❌ (direct)
- `query assignee` ❌ (direct)
- `query label` ❌ (direct)
- `query ready` ❌ (direct)
- `query blocked` ❌ (direct)
- `query strategic` ❌ (direct)
- `query closed` ❌ (direct)

#### Dependency Commands
- `dep add` (check)
- `dep rm` (check)

#### Gate Commands
- `gate define` (check)
- `gate add` (check)
- `gate pass` (check)
- `gate fail` (check)
- `gate check` (check)
- `gate check-all` (check)
- `gate list` ❌ (direct)
- `gate show` ❌ (direct)

#### Other Commands
- `graph show` ❌ (direct)
- `graph roots` ❌ (direct)
- `graph downstream` ❌ (direct)
- `doc add/remove/list/show` (check all)
- `events query/tail` ❌ (direct)
- `status` ❌ (direct)
- `validate` (check)

### 3. Update Error Handling

**Location**: `crates/jit/src/main.rs` main error handler

```rust
fn main() -> ExitCode {
    let cli = Cli::parse();
    
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            // If --json was passed, output JSON error
            if json_mode_active() {
                let response = JsonResponse::<()>::error(
                    error_code(&e),
                    &e.to_string(),
                    &current_command()
                );
                eprintln!("{}", serde_json::to_string_pretty(&response).unwrap());
            } else {
                eprintln!("Error: {:#}", e);
            }
            ExitCode::from(1)
        }
    }
}
```

**Helper functions**:
```rust
fn json_mode_active() -> bool {
    // Check if --json flag was passed
    std::env::args().any(|arg| arg == "--json")
}

fn error_code(err: &anyhow::Error) -> &str {
    // Map error types to codes
    // Use downcast or error message parsing
    "UNKNOWN_ERROR"
}

fn current_command() -> String {
    // Parse from CLI args
    std::env::args().nth(1).unwrap_or_default()
}
```

### 4. Add Tests

**File**: `crates/jit/tests/json_output_consistency_tests.rs`

```rust
#[test]
fn test_all_commands_use_standard_json_format() {
    let h = TestHarness::new();
    
    // Test issue commands
    let output = h.run_command(&["issue", "list", "--json"]);
    verify_json_envelope(&output);
    
    let output = h.run_command(&["issue", "show", &issue_id, "--json"]);
    verify_json_envelope(&output);
    
    // Test query commands
    let output = h.run_command(&["query", "ready", "--json"]);
    verify_json_envelope(&output);
    
    // ... test all commands
}

fn verify_json_envelope(output: &str) -> serde_json::Value {
    let json: serde_json::Value = serde_json::from_str(output)
        .expect("Output should be valid JSON");
    
    // Verify envelope structure
    assert!(json.get("success").is_some(), "Missing 'success' field");
    assert!(json.get("meta").is_some(), "Missing 'meta' field");
    
    let success = json["success"].as_bool().unwrap();
    if success {
        assert!(json.get("data").is_some(), "Success response missing 'data'");
    } else {
        assert!(json.get("error").is_some(), "Error response missing 'error'");
    }
    
    // Verify meta structure
    let meta = &json["meta"];
    assert!(meta.get("timestamp").is_some());
    assert!(meta.get("version").is_some());
    assert!(meta.get("command").is_some());
    
    json
}

#[test]
fn test_error_responses_use_standard_format() {
    let h = TestHarness::new();
    
    // Trigger error: non-existent issue
    let output = h.run_command_expect_error(&["issue", "show", "nonexistent", "--json"]);
    let json = verify_json_envelope(&output);
    
    assert_eq!(json["success"], false);
    assert!(json["error"]["code"].is_string());
    assert!(json["error"]["message"].is_string());
}

#[test]
fn test_json_output_is_valid_and_parseable() {
    let h = TestHarness::new();
    let issue = h.create_issue("Test");
    
    // Every JSON command should produce valid, parseable JSON
    let commands = vec![
        vec!["issue", "show", &issue, "--json"],
        vec!["issue", "list", "--json"],
        vec!["query", "ready", "--json"],
        vec!["status", "--json"],
    ];
    
    for cmd in commands {
        let output = h.run_command(&cmd);
        serde_json::from_str::<serde_json::Value>(&output)
            .expect(&format!("Command {:?} produced invalid JSON", cmd));
    }
}
```

### 5. Update MCP Server Integration

**File**: `mcp-server/src/main.rs`

Update to work with new consistent format:

```rust
// Now all responses have the same shape
fn handle_jit_command(args: &[&str]) -> Result<serde_json::Value> {
    let output = run_jit_cli(args)?;
    let response: JsonResponse<serde_json::Value> = serde_json::from_str(&output)?;
    
    if response.success {
        Ok(response.data.unwrap())
    } else {
        Err(anyhow!("{}", response.error.unwrap().message))
    }
}
```

### 6. Update Documentation

**Files**:
- `README.md` - Update JSON output examples
- `docs/design.md` - Document JSON response format
- `EXAMPLE.md` - Show JSON usage patterns

**Content**:
```markdown
## JSON Output

All commands support `--json` flag for machine-readable output with a consistent envelope format:

### Success Response
```json
{
  "success": true,
  "data": { /* command-specific data */ },
  "meta": {
    "timestamp": "2025-12-21T20:00:00Z",
    "version": "1.0.0",
    "command": "issue show"
  }
}
```

### Error Response
```json
{
  "success": false,
  "error": {
    "code": "ISSUE_NOT_FOUND",
    "message": "Issue not found: abc123"
  },
  "meta": {
    "timestamp": "2025-12-21T20:00:00Z",
    "version": "1.0.0",
    "command": "issue show"
  }
}
```

### Usage in Scripts
```bash
# Check success
if jq -e '.success' response.json; then
  # Extract data
  jq '.data' response.json
else
  # Handle error
  jq '.error.message' response.json
fi
```
```

## Implementation Approach

1. **Create base types** in `output.rs` (1 hour)
2. **Update 5-10 commands** as proof of concept (2 hours)
3. **Run tests**, ensure backward compatibility for non-JSON mode (1 hour)
4. **Systematically update** all remaining commands (4-5 hours)
5. **Update error handling** in main (1 hour)
6. **Add comprehensive tests** (2 hours)
7. **Update MCP server** (1 hour)
8. **Update documentation** (1 hour)
9. **Full test suite** + clippy + fmt (1 hour)

**Total: 14-16 hours**

## Migration Strategy

### Backward Compatibility
- Non-JSON mode (default) unchanged - no breaking changes
- Only affects `--json` output
- Existing scripts using `--json` may need updates

### Versioning
- Bump version to indicate JSON format change
- Document in CHANGELOG
- Consider `--json-v1` flag for legacy format (optional)

### Rollout
1. Update core commands first (issue, query)
2. Update peripheral commands
3. Test MCP server integration
4. Release with clear migration notes

## Success Criteria

✅ All commands with `--json` use identical envelope format  
✅ Success responses have `success: true, data: {}`  
✅ Error responses have `success: false, error: {}`  
✅ All responses include `meta` with timestamp, version, command  
✅ Tests verify format consistency across all commands  
✅ MCP server works with new format  
✅ Documentation updated  
✅ Zero clippy warnings  
✅ All existing tests pass  

## Benefits

- **Easier scripting**: Single parsing logic for all commands
- **Better MCP integration**: Consistent response handling
- **Error handling**: Structured error codes and messages
- **Debugging**: Timestamp and version in every response
- **Type safety**: Strongly typed response structures
- **Future-proof**: Easy to add fields without breaking changes

## Dependencies

- No new crate dependencies
- Uses existing: `serde`, `serde_json`, `chrono`

## Related Issues

- Improves agent-friendly CLI usability
- Enables better MCP tooling
- Supports scripting and automation use cases
