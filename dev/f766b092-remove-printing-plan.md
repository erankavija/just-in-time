# Remove direct printing from CommandExecutor (f766b092)

## Status: In Progress (Phase 2 of 5+ complete)

## Current Progress

### ✅ Phase 1 Complete: validate.rs status()
- Removed status() method that printed 7 lines directly
- Updated main.rs to use get_status() and format output with OutputContext
- StatusSummary already had Serialize derive
- Tested: both 'jit status' and 'jit status --json' work correctly
- Committed in: 5b87ef6

### ✅ Phase 2 Complete: document.rs list operations
- Removed printing from list_document_references(), document_history(), list_document_assets(), check_document_links()
- Created return structs: DocumentListResult, DocumentHistory, AssetListResult, LinkCheckResult, LinkCheckSummary
- Updated main.rs to format output using OutputContext and JsonOutput patterns
- Updated tests to expect wrapped JSON output (data["commits"] instead of raw array)
- Fixed pre-existing broken doctests in query_engine (jit::query -> jit::query_engine)
- All tests pass, zero clippy warnings

### 🔄 Next: Phase 3 - Warnings pattern
Focus on methods that use eprintln! for warnings (issue.rs, gate.rs, etc.)

## Audit of Remaining println!/eprintln! in commands/

### High Priority (mentioned in issue):
- [x] validate.rs: status() - DONE (Phase 1)
- [x] document.rs: document_history() and list_document_references() - DONE (Phase 2)
- [x] document.rs: list_document_assets() and check_document_links() - DONE (Phase 2)
- [ ] issue.rs: create_issue() and update_issue() use eprintln! for warnings
- [ ] mod.rs: require_active_lease() uses eprintln!

### Additional Files Found:
- [ ] gate.rs - 7 eprintln! warning calls
- [ ] gate_check.rs - 1 eprintln! error call
- [ ] labels.rs - 1 eprintln! warning call
- [ ] dependency.rs - 2 eprintln! warning calls
- [ ] claim.rs - 1 eprintln! error call
- [ ] snapshot.rs - println! for success messages + eprintln! warning
- [ ] validate.rs - still has println!s in validate() method (different from status)

## Strategy for Remaining Work

### Pattern 1: List/Display Commands
Methods that print formatted data should return structured data:
- Return a serializable struct
- Main.rs handles formatting for human/JSON output
- Example: validate.rs status() (completed)

### Pattern 2: Warnings
Methods that print warnings should return warnings as data:
- Return `Result<(T, Vec<String>)>` where Vec<String> is warnings
- Main.rs prints warnings using OutputContext::print_warning()
- Affected: issue.rs, gate.rs, labels.rs, dependency.rs

### Pattern 3: Error Messages
Methods using eprintln! for errors should use Result<T, E>:
- Convert eprintln! to proper error returns
- Let main.rs handle error display

## Detailed Plan for Remaining Phases

### Phase 2: document.rs list operations
**Target methods:**
- `list_document_references()` - prints document list
- `document_history()` - prints git history
- `list_assets()` - prints asset list
- `check_links()` - prints validation results

**Approach:**
- Create return structs (DocumentList, HistoryList, AssetList, LinkCheckResults)
- Add Serialize derives
- Update main.rs to format output

### Phase 3: Warnings pattern
**Target methods:**
- issue.rs: create_issue(), update_issue_general()
- gate.rs: multiple methods (7 sites)
- labels.rs: label operations
- dependency.rs: add_dependency(), remove_dependency()

**Approach:**
- Change return type to include warnings: `Result<(T, Vec<String>)>`
- require_active_lease() already returns Option<String> - use that
- Collect warnings and return them
- Main.rs displays warnings before success message

### Phase 4: validate.rs remaining prints
- validate() method has multiple println!s
- Similar to status(), create ValidationResult struct
- Return structured data

### Phase 5: Success messages (snapshot.rs, document.rs archive)
- Move success messages to main.rs
- Return operation summaries

## Testing Strategy

For each phase:
1. Run affected tests
2. Manual testing of commands
3. Test both human and JSON output
4. Verify zero new clippy warnings

## Notes

- This is substantial incremental work - do NOT rush
- Each phase should be tested and committed separately
- StatusSummary pattern (Phase 1) is good template
- Some methods already support JSON via json parameter - use that as guide
- Focus on clean separation: commands/ returns data, main.rs formats
