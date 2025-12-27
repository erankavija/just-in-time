# Snapshot Export Implementation Plan

**Issue:** #a8f2f04b - Implement snapshot export (basic)  
**Epic:** #71373e37 - Documentation Lifecycle and Reorganization  
**Phase:** 1 - Foundations  
**Estimated Effort:** 1-2 weeks

## Overview

Implement portable snapshot export that captures complete issue state with all linked documents and assets at a specific point in time. Critical for handoffs, audits, compliance, and reproducibility.

## Authoritative References

- **Design:** `dev/active/documentation-lifecycle-design.md` §10 (Snapshot Export)
- **Issue:** #a8f2f04b acceptance criteria
- **Related:** Asset scanning foundation from #896ff7df (doc archive)

## Goals

1. **Non-mutating:** Read-only operations, never modify repository
2. **Reproducible:** Same commit → same snapshot (deterministic)
3. **Self-contained:** Snapshot includes all data needed for verification
4. **Safe:** Validate repository integrity before export
5. **Portable:** Works as tar archive or directory structure

## Command Surface

```bash
jit snapshot export [OPTIONS]

Options:
  --out <PATH>              Output path (default: snapshot-YYYYMMDD-HHMMSS)
  --format <FORMAT>         Output format: dir | tar (default: dir)
  --scope <SCOPE>           Scope: all | issue:ID | epic:ID (default: all)
  --at <COMMIT>             Git commit/tag to export (requires git)
  --working-tree            Export from working tree instead of git
  --committed-only          Reject if uncommitted docs/assets (requires git, implies --at HEAD)
  --force                   Skip repository validation
  --json                    Output metadata in JSON
```

**Git Behavior:**
- **Default (no --at, no --working-tree):** Use git if available (export from HEAD), fall back to working tree
- **--at <commit>:** Requires git, exports from specified commit
- **--working-tree:** Never uses git, reads from filesystem
- **--committed-only:** Requires git, validates files match HEAD

## Snapshot Structure

```
snapshot-20251227-220000/
├── manifest.json          # Manifest v1 with provenance
├── README.md             # Auto-generated guide
├── .jit/                 # JIT state in scope
│   ├── config.toml       # Repository configuration
│   ├── gates.json        # Gate registry
│   └── issues/           # Issue JSON files
│       ├── abc123.json
│       └── def456.json
├── docs/                 # Documents (mirrored repo layout)
│   └── design/
│       └── feature.md
└── dev/                  # Development docs
    ├── active/
    │   ├── design.md
    │   └── design_assets/
    │       └── diagram.png
    └── studies/
        └── analysis.md
```

## Manifest Format (Version 1)

Based on design §10, manifest.json provides complete provenance and verification data:

```json
{
  "version": "1",
  "created_at": "2025-12-27T22:00:00Z",
  "created_by": "jit snapshot export v0.2.0",
  "repo": {
    "path": "/home/user/project",
    "remote": "https://github.com/user/project.git",
    "commit": "abc123def456...",     // null if --working-tree
    "branch": "main",                // null if --working-tree or detached
    "dirty": false,                  // always false with --committed-only
    "source": "git"                  // "git" | "working-tree"
  },
  "scope": "all",
  "issues": {
    "count": 10,
    "states": {
      "backlog": 2,
      "ready": 3,
      "in_progress": 1,
      "done": 4
    },
    "files": [
      ".jit/issues/abc123.json",
      ".jit/issues/def456.json"
    ]
  },
  "documents": {
    "count": 15,
    "items": [
      {
        "path": "dev/active/webhooks.md",
        "format": "markdown",
        "size_bytes": 4096,
        "source": {
          "type": "git",                    // "git" | "filesystem"
          "commit": "abc123",               // null if filesystem
          "blob_sha": "deadbeef"            // null if filesystem
        },
        "hash_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "assets": [
          {
            "path": "dev/active/webhooks_assets/flow.png",
            "source": {
              "type": "git",                // "git" | "filesystem"
              "commit": "abc123",           // null if filesystem  
              "blob_sha": "cafebabe"        // null if filesystem
            },
            "hash_sha256": "...",
            "mime": "image/png",
            "size_bytes": 12345,
            "shared": false
          }
        ]
      }
    ]
  },
  "metadata": {
    "link_policy": "preserve",
    "external_assets_policy": "exclude",
    "lfs_policy": "allow-pointers"
  },
  "verification": {
    "total_files": 25,
    "total_bytes": 102400,
    "instructions": "Run 'sha256sum -c checksums.txt' to verify integrity"
  }
}
```

## Auto-Generated README.md

```markdown
# JIT Snapshot Export

**Created:** 2025-12-27 22:00:00 UTC  
**Repository:** /home/user/project  
**Commit:** abc123def456...  
**Scope:** all

## Contents

This snapshot contains:
- 10 issues (.jit/issues/)
- 15 documents with associated assets
- Complete JIT configuration and gate definitions

## Verification

To verify snapshot integrity:

\`\`\`bash
# Verify all file hashes match manifest
jq -r '.documents.items[] | .path + " " + .hash_sha256' manifest.json | sha256sum -c

# Or use the provided checksums file
sha256sum -c checksums.txt
\`\`\`

## Structure

- `.jit/` - JIT repository state
- `docs/` - Product documentation
- `dev/` - Development documentation
- `manifest.json` - Complete provenance and verification data
- `checksums.txt` - SHA256 checksums for all files

## Import (Future)

This is a read-only export. Future versions may support:
- `jit snapshot import` - Restore snapshot to working repository
- `jit snapshot diff` - Compare snapshots
```

## Implementation Phases

### Phase 0: Validation & Setup

**Validation Check (unless --force):**
```rust
fn validate_before_export(&self) -> Result<()> {
    // Run jit validate to ensure repository integrity
    let validation = self.storage.validate_repository()?;
    
    if !validation.is_valid() {
        return Err(anyhow!(
            "Repository validation failed. Fix issues or use --force:\n{}",
            validation.format_errors()
        ));
    }
    
    Ok(())
}
```

**Source Mode Detection:**
```rust
enum SourceMode {
    Git { commit: String },      // Read from git at specific commit
    WorkingTree,                 // Read from filesystem
}

fn determine_source_mode(
    at_commit: Option<&str>,
    working_tree: bool,
    committed_only: bool,
) -> Result<SourceMode> {
    match (at_commit, working_tree, committed_only) {
        (Some(_), true, _) => {
            Err(anyhow!("Cannot use both --at and --working-tree"))
        }
        (Some(commit), _, _) => {
            // Explicit commit requires git
            if Repository::open(".").is_err() {
                return Err(anyhow!("--at requires git repository"));
            }
            Ok(SourceMode::Git { commit: commit.to_string() })
        }
        (_, true, _) => {
            // Explicit working tree - no git needed
            Ok(SourceMode::WorkingTree)
        }
        (None, false, true) => {
            // --committed-only implies --at HEAD
            if Repository::open(".").is_err() {
                return Err(anyhow!("--committed-only requires git repository"));
            }
            Ok(SourceMode::Git { commit: "HEAD".to_string() })
        }
        (None, false, false) => {
            // Default: use git if available, else working tree
            if let Ok(repo) = Repository::open(".") {
                let head = repo.head()?.peel_to_commit()?;
                Ok(SourceMode::Git { commit: head.id().to_string() })
            } else {
                Ok(SourceMode::WorkingTree)
            }
        }
    }
}
```

**Committed-Only Check (git mode only):**
```rust
fn check_committed_only(&self, docs: &[DocumentRef], mode: &SourceMode) -> Result<()> {
    let SourceMode::Git { commit } = mode else {
        return Ok(()); // Not applicable for working tree mode
    };
    
    if commit != "HEAD" {
        return Ok(()); // Not checking historical commits
    }
    
    let repo = Repository::open(".")?;
    
    for doc in docs {
        // Check if file exists in working tree but differs from HEAD
        let working_tree_hash = compute_file_hash(&doc.path)?;
        let head_hash = read_from_git_and_hash(&repo, &doc.path, "HEAD")?;
        
        if working_tree_hash != head_hash {
            return Err(anyhow!(
                "Uncommitted changes in {}\nCommit changes or remove --committed-only flag",
                doc.path
            ));
        }
    }
    
    Ok(())
}
```

### Phase 1: Scope Enumeration

**Query issues based on scope:**

```rust
fn enumerate_issues(&self, scope: &SnapshotScope) -> Result<Vec<Issue>> {
    match scope {
        SnapshotScope::All => {
            self.storage.list_issues()
        }
        SnapshotScope::Issue(id) => {
            let issue = self.storage.load_issue(id)?;
            Ok(vec![issue])
        }
        SnapshotScope::Epic(id) => {
            // Get epic and all downstream dependencies
            let issues = self.storage.list_issues()?;
            let graph = DependencyGraph::new(&issues.iter().collect::<Vec<_>>());
            let downstream = graph.get_downstream(id);
            
            Ok(downstream.into_iter().cloned().collect())
        }
    }
}
```

**Extract document references:**

```rust
fn enumerate_documents(&self, issues: &[Issue]) -> Result<Vec<DocumentSnapshot>> {
    let mut docs = Vec::new();
    
    for issue in issues {
        for doc_ref in &issue.documents {
            let snapshot = self.create_document_snapshot(doc_ref)?;
            docs.push(snapshot);
        }
    }
    
    // Deduplicate by path
    docs.sort_by(|a, b| a.path.cmp(&b.path));
    docs.dedup_by(|a, b| a.path == b.path);
    
    Ok(docs)
}
```

### Phase 2: Content Extraction

**Read content based on source mode:**

```rust
fn read_content(
    &self,
    path: &str,
    mode: &SourceMode,
) -> Result<(Vec<u8>, SourceInfo)> {
    match mode {
        SourceMode::Git { commit } => {
            let repo = Repository::open(".")?;
            self.read_from_git(&repo, path, commit)
        }
        SourceMode::WorkingTree => {
            self.read_from_filesystem(path)
        }
    }
}

fn read_from_git(
    &self,
    repo: &Repository,
    path: &str,
    reference: &str,
) -> Result<(Vec<u8>, SourceInfo)> {
    // reference is "HEAD", commit SHA, or tag
    let obj = repo.revparse_single(reference)?;
    let commit = obj.peel_to_commit()?;
    let tree = commit.tree()?;
    
    let entry = tree.get_path(Path::new(path))?;
    let blob = repo.find_blob(entry.id())?;
    
    let content = blob.content().to_vec();
    
    Ok((content, SourceInfo {
        type_: "git".to_string(),
        commit: Some(commit.id().to_string()),
        blob_sha: Some(entry.id().to_string()),
    }))
}

fn read_from_filesystem(&self, path: &str) -> Result<(Vec<u8>, SourceInfo)> {
    use std::fs;
    
    let full_path = self.storage.root().parent()
        .ok_or_else(|| anyhow!("Invalid storage path"))?
        .join(path);
    
    let content = fs::read(&full_path)
        .with_context(|| format!("Failed to read file: {}", path))?;
    
    Ok((content, SourceInfo {
        type_: "filesystem".to_string(),
        commit: None,
        blob_sha: None,
    }))
}
```

**Compute SHA256:**

```rust
use sha2::{Sha256, Digest};

fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}
```

**Create document snapshot:**

```rust
fn create_document_snapshot(
    &self,
    doc_ref: &DocumentRef,
    mode: &SourceMode,
) -> Result<DocumentSnapshot> {
    // Read document content
    let (content, source) = self.read_content(&doc_ref.path, mode)?;
    let hash = compute_sha256(&content);
    
    // Get assets (from metadata or re-scan)
    let assets = if let Some(ref asset_list) = doc_ref.assets {
        asset_list.clone()
    } else {
        // Fallback: re-scan
        self.scan_assets(&doc_ref.path, &String::from_utf8_lossy(&content))?
    };
    
    // Create asset snapshots
    let asset_snapshots = assets.iter()
        .map(|asset| self.create_asset_snapshot(asset, mode))
        .collect::<Result<Vec<_>>>()?;
    
    Ok(DocumentSnapshot {
        path: doc_ref.path.clone(),
        format: doc_ref.format.clone().unwrap_or_else(|| "markdown".to_string()),
        size_bytes: content.len(),
        source,
        hash_sha256: hash,
        assets: asset_snapshots,
    })
}
```

### Phase 3: Assembly

**Create temp directory:**

```rust
fn create_snapshot_assembly(&self, issues: &[Issue], docs: &[DocumentSnapshot]) -> Result<SnapshotAssembly> {
    let temp_dir = TempDir::new()?;
    let base = temp_dir.path();
    
    // Create directory structure
    fs::create_dir_all(base.join(".jit/issues"))?;
    fs::create_dir_all(base.join("docs"))?;
    fs::create_dir_all(base.join("dev"))?;
    
    // Copy .jit state files
    self.copy_jit_state(base, issues)?;
    
    // Extract and write documents and assets
    for doc in docs {
        self.write_document_to_snapshot(base, doc)?;
    }
    
    // Generate manifest
    let manifest = self.generate_manifest(issues, docs)?;
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::write(base.join("manifest.json"), manifest_json)?;
    
    // Generate README
    let readme = self.generate_readme(&manifest)?;
    fs::write(base.join("README.md"), readme)?;
    
    // Generate checksums.txt
    self.generate_checksums_file(base)?;
    
    Ok(SnapshotAssembly {
        temp_dir,
        manifest,
    })
}
```

**Write document:**

```rust
fn write_document_to_snapshot(&self, base: &Path, doc: &DocumentSnapshot) -> Result<()> {
    let repo = Repository::open(".")?;
    
    // Read content from git
    let (content, _) = self.read_document_from_git(&repo, &doc.path, &doc.source.commit)?;
    
    // Write to snapshot preserving path
    let dest_path = base.join(&doc.path);
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&dest_path, content)?;
    
    // Write assets
    for asset in &doc.assets {
        let (asset_content, _) = self.read_document_from_git(&repo, &asset.path, &asset.source.commit)?;
        let asset_dest = base.join(&asset.path);
        if let Some(parent) = asset_dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&asset_dest, asset_content)?;
    }
    
    Ok(())
}
```

### Phase 4: Packaging

**Directory format:**

```rust
fn package_as_directory(&self, assembly: SnapshotAssembly, out_path: &Path) -> Result<()> {
    // Atomic rename from temp to final destination
    fs::rename(assembly.temp_dir.path(), out_path)?;
    
    println!("✓ Snapshot exported to: {}", out_path.display());
    println!("  {} issues", assembly.manifest.issues.count);
    println!("  {} documents", assembly.manifest.documents.count);
    
    Ok(())
}
```

**TAR format:**

```rust
fn package_as_tar(&self, assembly: SnapshotAssembly, out_path: &Path) -> Result<()> {
    use tar::Builder;
    use std::fs::File;
    
    let tar_file = File::create(out_path)?;
    let mut tar = Builder::new(tar_file);
    
    // Add all files from temp directory
    tar.append_dir_all(".", assembly.temp_dir.path())?;
    tar.finish()?;
    
    println!("✓ Snapshot exported to: {}", out_path.display());
    println!("  {} issues", assembly.manifest.issues.count);
    println!("  {} documents", assembly.manifest.documents.count);
    println!("  Archive: {}", out_path.display());
    
    Ok(())
}
```

## Data Structures

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotManifest {
    pub version: String,
    pub created_at: String,
    pub created_by: String,
    pub repo: RepoInfo,
    pub scope: String,
    pub issues: IssuesInfo,
    pub documents: DocumentsInfo,
    pub metadata: MetadataInfo,
    pub verification: VerificationInfo,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoInfo {
    pub path: String,
    pub remote: Option<String>,
    pub commit: Option<String>,        // null if working-tree mode
    pub branch: Option<String>,        // null if working-tree or detached
    pub dirty: bool,
    pub source: String,                // "git" | "working-tree"
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DocumentSnapshot {
    pub path: String,
    pub format: String,
    pub size_bytes: usize,
    pub source: SourceInfo,
    pub hash_sha256: String,
    pub assets: Vec<AssetSnapshot>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceInfo {
    #[serde(rename = "type")]
    pub type_: String,                 // "git" | "filesystem"
    pub commit: Option<String>,        // null if filesystem
    pub blob_sha: Option<String>,      // null if filesystem
}

pub enum SnapshotScope {
    All,
    Issue(String),
    Epic(String),
}

pub enum SnapshotFormat {
    Directory,
    Tar,
}

pub enum SourceMode {
    Git { commit: String },
    WorkingTree,
}
```

## Git-Optional Design

**Key Principle:** Snapshot export should work with or without git, following jit's core philosophy that git is optional.

**Behavior Matrix:**

| Flags | Git Available | Behavior |
|-------|---------------|----------|
| (none) | Yes | Export from HEAD |
| (none) | No | Export from working tree |
| `--at <commit>` | Yes | Export from specified commit |
| `--at <commit>` | No | **ERROR**: requires git |
| `--working-tree` | Yes | Export from working tree (ignore git) |
| `--working-tree` | No | Export from working tree |
| `--committed-only` | Yes | Export from HEAD, validate no uncommitted |
| `--committed-only` | No | **ERROR**: requires git |

**Manifest Differences:**

**Git mode:**
```json
"source": {
  "type": "git",
  "commit": "abc123",
  "blob_sha": "deadbeef"
}
```

**Working tree mode:**
```json
"source": {
  "type": "filesystem",
  "commit": null,
  "blob_sha": null
}
```

**Reproducibility:**
- Git mode: Reproducible (same commit → same snapshot)
- Working tree mode: NOT reproducible (working tree can change)

## Testing Strategy

### Unit Tests

```rust
#[test]
fn test_compute_sha256() {
    let data = b"hello world";
    let hash = compute_sha256(data);
    assert_eq!(hash, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
}

#[test]
fn test_enumerate_scope_all() {
    // Test that all issues are returned
}

#[test]
fn test_enumerate_scope_issue() {
    // Test single issue enumeration
}

#[test]
fn test_enumerate_scope_epic() {
    // Test epic + downstream issues
}
```

### Integration Tests

```rust
#[test]
fn test_export_all_to_dir() {
    // Create test repo with issues and docs
    // Export with --scope all --format dir
    // Verify structure exists
    // Verify manifest.json is valid
    // Verify checksums match
}

#[test]
fn test_export_single_issue_to_tar() {
    // Export single issue
    // Verify tar contains only that issue's docs
}

#[test]
fn test_export_epic_with_dependencies() {
    // Create epic with downstream issues
    // Export epic scope
    // Verify all dependencies included
}

#[test]
fn test_manifest_hashes_match_content() {
    // Export snapshot
    // Read manifest
    // Verify every SHA256 in manifest matches actual file
}

#[test]
fn test_committed_only_rejects_uncommitted() {
    // Create uncommitted changes
    // Export with --committed-only
    // Should fail with clear error
}

#[test]
fn test_missing_assets_handled() {
    // Create issue with missing asset reference
    // Export should warn but not fail
    // Manifest should note missing assets
}

#[test]
fn test_validation_fails_for_invalid_repo() {
    // Create invalid state (missing issue file)
    // Export without --force should fail
    // Export with --force should succeed
}
```

## Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("Repository validation failed: {0}")]
    ValidationFailed(String),
    
    #[error("Uncommitted changes in {path}. Commit changes or remove --committed-only")]
    UncommittedChanges { path: String },
    
    #[error("Asset not found in git: {path} at {commit}")]
    AssetNotFound { path: String, commit: String },
    
    #[error("Invalid scope format: {0}")]
    InvalidScope(String),
    
    #[error("Output path already exists: {0}")]
    OutputExists(String),
}
```

## CLI Help Text

```
Archive a complete snapshot of issues and documents

Usage: jit snapshot export [OPTIONS]

Options:
      --out <PATH>       Output path (default: snapshot-YYYYMMDD-HHMMSS)
      --format <FORMAT>  Output format: dir, tar [default: dir]
      --scope <SCOPE>    Scope: all, issue:ID, epic:ID [default: all]
      --committed-only   Reject if uncommitted docs/assets exist
      --force            Skip repository validation
      --json             Output metadata in JSON
  -h, --help             Print help
```

## Implementation Checklist

### Phase 0: Foundation
- [ ] Define data structures (SnapshotManifest, etc.)
- [ ] Write unit tests for SHA256 computation
- [ ] Write unit tests for scope enumeration

### Phase 1: Validation
- [ ] Implement validation check (unless --force)
- [ ] Implement committed-only check
- [ ] Test validation error handling

### Phase 2: Enumeration
- [ ] Implement scope parsing (all/issue:ID/epic:ID)
- [ ] Implement issue enumeration by scope
- [ ] Implement document extraction from issues
- [ ] Test scope filtering

### Phase 3: Extraction
- [ ] Implement read_document_from_git
- [ ] Implement read_binary_from_git
- [ ] Implement SHA256 computation
- [ ] Implement document snapshot creation
- [ ] Test content extraction

### Phase 4: Assembly
- [ ] Create temp directory structure
- [ ] Copy .jit state files
- [ ] Write documents and assets
- [ ] Generate manifest.json
- [ ] Generate README.md
- [ ] Generate checksums.txt
- [ ] Test assembly phase

### Phase 5: Packaging
- [ ] Implement directory output (atomic rename)
- [ ] Implement tar output
- [ ] Test both formats
- [ ] Verify reproducibility

### Phase 6: Integration
- [ ] All 7 integration tests passing
- [ ] CLI help text finalized
- [ ] Error messages clear and actionable

### Phase 7: Quality Gates
- [ ] TDD reminder passed
- [ ] All tests passing
- [ ] Clippy clean
- [ ] Formatted
- [ ] Code review

## Success Criteria

From issue #a8f2f04b:

- ✅ Snapshot export functional for dir and tar
- ✅ Manifest complete with provenance
- ✅ Non-mutating (read-only)
- ✅ Reproducible (same commit → same snapshot)
- ✅ Repository validation before export (unless --force)
- ✅ Tests verify integrity
- ✅ Missing assets handled gracefully
- ✅ Committed-only mode works

## Open Questions

1. Should checksums.txt be a separate file or embedded in manifest?
   - **Decision:** Separate file for ease of `sha256sum -c` usage
   
2. How to handle missing assets?
   - **Decision:** Warn in output, note in manifest, continue export
   
3. Should we include gate run results in snapshots?
   - **Decision:** No for Phase 1 (basic), consider for Phase 2

## Notes

- Uses existing `read_document_content` for git operations
- Reuses asset scanning from doc archive implementation (#896ff7df)
- Temp directory pattern ensures atomic operations
- SHA256 provides verification and deduplication capability
- Validation ensures we don't export corrupt state
