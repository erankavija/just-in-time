# Issue 5e1d5f02: Add Configurable Enforcement Modes

**Status:** In Progress, Claimed  
**Lease:** a76a4f4a-a69a-4f44-8f72-181c1624e501 (600s)

## Context from Configuration Story (7051d24e)

The full configuration system story includes:
1. Repository config schema (`.jit/config.toml`)
2. Agent config (`~/.config/jit/agent.toml`) 
3. Worktree mode setting (`auto`/`on`/`off`)
4. **Enforcement mode setting** (`strict`/`warn`/`off`) ← THIS ISSUE
5. TTL and heartbeat settings
6. Config commands (`jit config show/get/set/validate`)
7. Environment variable overrides

**This issue is a subset** - Just the enforcement mode configuration piece.

## Design from worktree-parallel-work.md

### Required Config Structure

```toml
[worktree]
# strict: block operations without lease
# warn: warn but allow operations without lease  
# off: no lease enforcement
enforce_leases = "strict"  # default
```

### Enforcement Modes

1. **`strict`** - Block structural operations without active lease
   - Return error before operation
   - User must acquire lease first
   - Production-safe default

2. **`warn`** - Warn but allow operations without lease
   - Log warning to stderr
   - Allow operation to proceed
   - Development-friendly mode

3. **`off`** - No enforcement
   - Bypass all lease checks
   - Backward compatible with sequential workflows
   - For single-agent or legacy use

## Current Implementation

**Config files exist:**
- `crates/jit/src/config.rs` - Config structs with serde
- `crates/jit/src/config_manager.rs` - Config loading logic

**Current JitConfig structure:**
```rust
pub struct JitConfig {
    pub version: Option<VersionConfig>,
    pub type_hierarchy: Option<HierarchyConfigToml>,
    pub validation: Option<ValidationConfig>,
    pub documentation: Option<DocumentationConfig>,
    pub namespaces: Option<HashMap<String, NamespaceConfig>>,
}
```

**Missing:** `worktree` section with `enforce_leases` field

## Implementation Plan (TDD)

### 1. Add Config Structs (Test First)
```rust
// In config.rs
#[derive(Debug, Clone, Deserialize)]
pub struct WorktreeConfig {
    /// Lease enforcement mode: "strict", "warn", or "off"
    pub enforce_leases: Option<String>,
}

// Add to JitConfig
pub worktree: Option<WorktreeConfig>,
```

### 2. Add Validation & Defaults
```rust
impl WorktreeConfig {
    pub fn enforce_leases_mode(&self) -> EnforcementMode {
        // Parse string to enum, default to Strict
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EnforcementMode {
    Strict,
    Warn,
    Off,
}
```

### 3. Expose via ConfigManager
```rust
impl ConfigManager {
    pub fn get_enforcement_mode(&self) -> Result<EnforcementMode> {
        // Load config, extract mode, return with defaults
    }
}
```

### 4. Write Tests
- Test parsing all three modes from TOML
- Test default to `strict` when missing
- Test invalid mode returns error
- Test config loading integration

### 5. Usage in CLI Enforcement (Next Issue)
```rust
// In commands/mod.rs (next issue: 82b17394)
fn require_active_lease(&self, issue_id: &str) -> Result<()> {
    let mode = self.config_manager.get_enforcement_mode()?;
    match mode {
        EnforcementMode::Off => Ok(()),  // Bypass
        EnforcementMode::Warn => {
            if !has_active_lease(issue_id) {
                eprintln!("⚠️  Warning: No active lease for {}", issue_id);
            }
            Ok(())
        }
        EnforcementMode::Strict => {
            if !has_active_lease(issue_id) {
                anyhow::bail!("Lease required for {}. Run: jit claim acquire {}", issue_id, issue_id);
            }
            Ok(())
        }
    }
}
```

## Acceptance Criteria

- [x] Issue claimed and in progress
- [ ] WorktreeConfig struct added to config.rs
- [ ] EnforcementMode enum with Strict/Warn/Off
- [ ] Parsing from TOML with validation
- [ ] Default to "strict" when not specified
- [ ] Tests for all modes
- [ ] Tests for invalid mode errors
- [ ] ConfigManager exposes get_enforcement_mode()
- [ ] All tests pass
- [ ] Zero clippy warnings
- [ ] Documentation updated

## Notes

- Hooks are **optional** (user-installed) per user's requirement
- CLI enforcement is the **primary** safety mechanism
- Config file location: `<worktree>/.jit/config.toml`
- This is foundation for next issue 82b17394 (CLI enforcement)
