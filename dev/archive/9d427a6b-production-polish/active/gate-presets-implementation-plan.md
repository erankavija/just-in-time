# Gate Templates and Presets - Implementation Plan

**Issue:** 56b7e503  
**Goal:** Enable users to quickly apply pre-configured gate bundles for common workflows

## Scope

### In Scope
- Built-in presets: `rust-tdd` and `minimal`
- CLI commands: `list`, `show`, `apply`, `create`
- Custom preset support (`.jit/config/gate-presets/`)
- Customization flags: `--timeout`, `--no-precheck`, `--no-postcheck`, `--except`
- Comprehensive tests following TDD principles

### Out of Scope
- Additional language presets (python, js, security) - future work
- `--only` flag (can achieve same with single gate add)
- Web UI integration

## Implementation Plan

### Phase 1: Data Model & Core Types

**Files:** `crates/jit/src/domain.rs`, `crates/jit/src/gate_presets.rs` (new)

- [ ] Define `GatePresetDefinition` struct
  - `name: String`
  - `description: String`
  - `gates: Vec<GateTemplate>`
- [ ] Define `GateTemplate` struct
  - `key: String`
  - `title: String`
  - `description: String`
  - `stage: GateStage`
  - `mode: GateMode`
  - `checker: Option<GateChecker>` (with command, timeout, working_dir)
- [ ] Add serde derives for JSON serialization
- [ ] Add validation logic for preset structure

**Tests:**
- [ ] Test preset deserialization from JSON
- [ ] Test validation (required fields, valid stages/modes)
- [ ] Test invalid preset rejection

### Phase 2: Built-in Presets

**Files:** `crates/jit/src/gate_presets/builtin.rs` (new)

- [ ] Create `rust-tdd` preset JSON definition
  - tdd-reminder (manual, precheck)
  - tests (auto, postcheck, `cargo test`)
  - clippy (auto, postcheck, `cargo clippy -- -D warnings`)
  - fmt (auto, postcheck, `cargo fmt --check`)
  - code-review (manual, postcheck)
- [ ] Create `minimal` preset JSON definition
  - code-review (manual, postcheck)
- [ ] Embed presets using `include_str!` macro
- [ ] Create `BuiltinPresets::load()` function returning HashMap

**Tests:**
- [ ] Test both presets parse correctly
- [ ] Test builtin preset registry initialization
- [ ] Test preset lookup by name

### Phase 3: Preset Manager

**Files:** `crates/jit/src/gate_presets/manager.rs` (new)

- [ ] `PresetManager` struct with methods:
  - `new(jit_root: PathBuf)` - Initialize with paths
  - `load_all() -> Result<Vec<GatePresetDefinition>>` - Load builtin + custom
  - `get_preset(name: &str) -> Result<&GatePresetDefinition>` - Lookup by name
  - `list_presets() -> Vec<PresetInfo>` - Summary info for listing
- [ ] Custom preset loading from `.jit/config/gate-presets/*.json`
- [ ] Override logic: custom presets override builtin with same name
- [ ] Validation on load with clear error messages

**Tests:**
- [ ] Test loading builtin presets only
- [ ] Test loading custom preset from file
- [ ] Test custom preset overrides builtin
- [ ] Test invalid preset file handling
- [ ] Test missing directory handling (should not error)

### Phase 4: Preset Application Logic

**Files:** `crates/jit/src/gate_presets/apply.rs` (new)

- [ ] `PresetApplicator` with `apply()` method
  - Input: `issue_id`, `preset_name`, `options`
  - Apply gates from preset to issue
  - Handle customization options
- [ ] `PresetApplyOptions` struct:
  - `timeout_override: Option<u64>`
  - `skip_precheck: bool`
  - `skip_postcheck: bool`
  - `except_gates: Vec<String>`
- [ ] Filter gates based on options
- [ ] Apply timeout overrides
- [ ] Validate gates don't already exist on issue
- [ ] Use existing gate definition + gate add logic

**Tests:**
- [ ] Test applying preset to issue (integration)
- [ ] Test `--no-precheck` filters correctly
- [ ] Test `--no-postcheck` filters correctly
- [ ] Test `--except` excludes specific gates
- [ ] Test `--timeout` overrides all checker timeouts
- [ ] Test error when gate already exists
- [ ] Test applying to non-existent issue

### Phase 5: CLI Commands - `jit gate preset list`

**Files:** `crates/jit/src/commands/gate.rs`

- [ ] Add `preset` subcommand group
- [ ] Implement `list` command
  - Display: name, description, gate count
  - Indicate builtin vs custom
  - Support `--json` output
- [ ] Wire up to PresetManager

**Tests:**
- [ ] CLI test: `jit gate preset list` shows presets
- [ ] CLI test: JSON output is valid
- [ ] CLI test: Shows both builtin and custom

### Phase 6: CLI Commands - `jit gate preset show`

**Files:** `crates/jit/src/commands/gate.rs`

- [ ] Implement `show <preset-name>` command
  - Display full preset details
  - List all gates with their configurations
  - Support `--json` output
- [ ] Handle non-existent preset error

**Tests:**
- [ ] CLI test: Show rust-tdd preset details
- [ ] CLI test: Show minimal preset details
- [ ] CLI test: Error on invalid preset name
- [ ] CLI test: JSON output matches schema

### Phase 7: CLI Commands - `jit gate preset apply`

**Files:** `crates/jit/src/commands/gate.rs`

- [ ] Implement `apply <issue-id> <preset-name>` command
- [ ] Add flags: `--timeout`, `--no-precheck`, `--no-postcheck`, `--except`
- [ ] Call PresetApplicator with options
- [ ] Display success message with gates applied
- [ ] Support `--json` output

**Tests:**
- [ ] CLI test: Apply rust-tdd preset to issue
- [ ] CLI test: Apply with --no-precheck
- [ ] CLI test: Apply with --no-postcheck
- [ ] CLI test: Apply with --except clippy
- [ ] CLI test: Apply with --timeout 120
- [ ] CLI test: Error on non-existent issue
- [ ] CLI test: Error on non-existent preset
- [ ] Verify gates actually added to issue

### Phase 8: CLI Commands - `jit gate preset create`

**Files:** `crates/jit/src/commands/gate.rs`

- [ ] Implement `create <name> --from-issue <issue-id>` command
- [ ] Extract gates from issue
- [ ] Generate preset JSON
- [ ] Save to `.jit/config/gate-presets/<name>.json`
- [ ] Create directory if needed
- [ ] Validate name (no special chars, no conflicts with builtin)
- [ ] Support `--json` output

**Tests:**
- [ ] CLI test: Create preset from issue with gates
- [ ] CLI test: Verify file created in correct location
- [ ] CLI test: Load and apply created preset
- [ ] CLI test: Error on duplicate name
- [ ] CLI test: Error on issue with no gates
- [ ] CLI test: Error on non-existent issue

### Phase 9: Integration & Polish

- [ ] Run full test suite: `cargo test --workspace`
- [ ] Run clippy: `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] Run fmt: `cargo fmt --all --check`
- [ ] Test real workflow end-to-end manually:
  - List presets
  - Show rust-tdd preset
  - Create issue
  - Apply rust-tdd preset
  - Verify gates added correctly
  - Create custom preset from issue
  - Apply custom preset to new issue
- [ ] Update `--help` text for all commands
- [ ] Ensure error messages are clear and actionable

### Phase 10: Documentation

**Files:** `docs/how-to/custom-gates.md`, `docs/reference/cli-commands.md`

- [ ] Add "Gate Presets and Templates" section to custom-gates.md
  - Replace placeholder with actual implementation
  - Include examples of all commands
  - Document both builtin presets
- [ ] Update CLI commands reference
  - Document `jit gate preset list`
  - Document `jit gate preset show`
  - Document `jit gate preset apply` with all flags
  - Document `jit gate preset create`
- [ ] Add example workflows
  - Quick start with rust-tdd
  - Creating custom preset
  - Applying with customizations

## Acceptance Criteria

- ✅ All tests pass (unit, integration, CLI)
- ✅ Clippy passes with zero warnings
- ✅ Code formatted with `cargo fmt`
- ✅ Both builtin presets (rust-tdd, minimal) work correctly
- ✅ Custom presets load from `.jit/config/gate-presets/`
- ✅ All 4 CLI commands function properly
- ✅ All customization flags work as designed
- ✅ Documentation updated with working examples
- ✅ Manual end-to-end test passes

## Notes

- Follow TDD: Write tests first, implement minimal code to pass
- Use functional programming patterns where practical
- Leverage existing gate definition and gate add infrastructure
- Ensure atomic operations (preset application should be all-or-nothing)
- Provide clear error messages with context
- Support `--json` output on all commands for agent consumption
