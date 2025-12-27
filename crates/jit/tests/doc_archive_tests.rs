//! Integration tests for document archival
//!
//! Tests for `jit doc archive` command

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Test helper: Initialize a git repository with jit
struct TestRepo {
    #[allow(dead_code)]
    temp_dir: TempDir,
    repo_path: std::path::PathBuf,
}

impl TestRepo {
    fn new() -> Self {
        let temp_dir = TempDir::new().expect("create temp dir");
        let repo_path = temp_dir.path().to_path_buf();

        // Initialize git repo
        Self::run_git(&repo_path, &["init"]);
        Self::run_git(&repo_path, &["config", "user.name", "Test User"]);
        Self::run_git(&repo_path, &["config", "user.email", "test@example.com"]);

        Self {
            temp_dir,
            repo_path,
        }
    }

    fn run_git(path: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(path)
            .args(args)
            .status()
            .expect("git command failed");
        assert!(status.success(), "git command failed: {:?}", args);
    }

    fn write_file(&self, path: &str, content: &str) {
        let full_path = self.repo_path.join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(full_path, content).expect("write file");
    }

    fn commit(&self, message: &str) {
        Self::run_git(&self.repo_path, &["add", "-A"]);
        Self::run_git(&self.repo_path, &["commit", "-m", message]);
    }

    fn path(&self) -> &Path {
        &self.repo_path
    }

    fn init_jit(&self) {
        let status = Command::new(env!("CARGO_BIN_EXE_jit"))
            .current_dir(self.path())
            .arg("init")
            .status()
            .expect("jit init failed");
        assert!(status.success(), "jit init should succeed");
    }

    fn run_jit(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_jit"))
            .current_dir(self.path())
            .args(args)
            .output()
            .expect("jit command failed")
    }

    fn file_exists(&self, path: &str) -> bool {
        self.repo_path.join(path).exists()
    }

    fn make_readonly(&self, path: &str) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let full_path = self.repo_path.join(path);
            let mut perms = fs::metadata(&full_path).unwrap().permissions();
            perms.set_mode(0o444); // Read-only
            fs::set_permissions(&full_path, perms).unwrap();
        }
        #[cfg(not(unix))]
        {
            let full_path = self.repo_path.join(path);
            let mut perms = fs::metadata(&full_path).unwrap().permissions();
            perms.set_readonly(true);
            fs::set_permissions(&full_path, perms).unwrap();
        }
    }

    fn make_writable(&self, path: &str) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let full_path = self.repo_path.join(path);
            if let Ok(metadata) = fs::metadata(&full_path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o644); // Read-write
                let _ = fs::set_permissions(&full_path, perms);
            }
        }
        #[cfg(not(unix))]
        {
            let full_path = self.repo_path.join(path);
            if let Ok(metadata) = fs::metadata(&full_path) {
                let mut perms = metadata.permissions();
                perms.set_readonly(false);
                let _ = fs::set_permissions(&full_path, perms);
            }
        }
    }
}

#[test]
fn test_archive_doc_with_per_doc_assets() {
    let repo = TestRepo::new();
    repo.init_jit();

    // Configure archive categories
    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
"#,
    );
    repo.commit("Add config");

    // Create a document with per-doc assets
    repo.write_file(
        "dev/active/feature-x-design.md",
        r#"# Feature X Design

![Diagram](assets/diagram.png)
"#,
    );
    repo.write_file("dev/active/assets/diagram.png", "fake png data");
    repo.commit("Add feature X design with assets");

    // Archive the document (use category key "design", which maps to "features" subdirectory)
    let output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/active/feature-x-design.md",
        "--type",
        "design",
    ]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should succeed
    assert!(
        output.status.success(),
        "Archive should succeed\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    // Source document should be gone
    assert!(
        !repo.file_exists("dev/active/feature-x-design.md"),
        "Source document should be moved"
    );

    // Destination document should exist
    assert!(
        repo.file_exists("dev/archive/features/feature-x-design.md"),
        "Document should be in archive"
    );

    // Assets should be moved with the document
    assert!(
        !repo.file_exists("dev/active/assets/diagram.png"),
        "Source asset should be moved"
    );
    assert!(
        repo.file_exists("dev/archive/features/assets/diagram.png"),
        "Asset should be in archive with document"
    );
}

#[test]
fn test_archive_doc_with_doc_name_assets_pattern() {
    let repo = TestRepo::new();
    repo.init_jit();

    // Configure archive categories
    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
"#,
    );
    repo.commit("Add config");

    // Create a document with per-doc assets using <doc-name>_assets/ pattern
    repo.write_file(
        "dev/active/abcd-design.md",
        r#"# ABCD Design

![Architecture](abcd-design_assets/arch.png)
![Flow](abcd-design_assets/flow.svg)
"#,
    );
    repo.write_file("dev/active/abcd-design_assets/arch.png", "fake png");
    repo.write_file("dev/active/abcd-design_assets/flow.svg", "fake svg");
    repo.commit("Add ABCD design with named assets");

    // Archive the document
    let output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/active/abcd-design.md",
        "--type",
        "design",
    ]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should succeed
    assert!(
        output.status.success(),
        "Archive should succeed\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    // Source document and assets should be gone
    assert!(
        !repo.file_exists("dev/active/abcd-design.md"),
        "Source document should be moved"
    );
    assert!(
        !repo.file_exists("dev/active/abcd-design_assets/arch.png"),
        "Source asset arch.png should be moved"
    );
    assert!(
        !repo.file_exists("dev/active/abcd-design_assets/flow.svg"),
        "Source asset flow.svg should be moved"
    );

    // Destination document and assets should exist
    assert!(
        repo.file_exists("dev/archive/features/abcd-design.md"),
        "Document should be in archive"
    );
    assert!(
        repo.file_exists("dev/archive/features/abcd-design_assets/arch.png"),
        "Asset arch.png should be in archive"
    );
    assert!(
        repo.file_exists("dev/archive/features/abcd-design_assets/flow.svg"),
        "Asset flow.svg should be in archive"
    );
}

#[test]
fn test_archive_with_shared_root_relative_links() {
    let repo = TestRepo::new();
    repo.init_jit();

    // Configure archive categories
    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
"#,
    );
    repo.commit("Add config");

    // Create shared assets in docs/
    repo.write_file("docs/diagrams/shared-logo.png", "shared logo");
    repo.write_file("docs/images/banner.jpg", "shared banner");

    // Create document with root-relative links to shared assets
    repo.write_file(
        "dev/active/feature-y-design.md",
        r#"# Feature Y Design

Company logo: ![Logo](/docs/diagrams/shared-logo.png)

Banner: ![Banner](/docs/images/banner.jpg)
"#,
    );
    repo.commit("Add feature Y with root-relative links");

    // Archive the document - should succeed because root-relative links stay valid
    let output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/active/feature-y-design.md",
        "--type",
        "design",
    ]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should succeed
    assert!(
        output.status.success(),
        "Archive should succeed with root-relative links\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    // Document should be archived
    assert!(
        repo.file_exists("dev/archive/features/feature-y-design.md"),
        "Document should be archived"
    );

    // Shared assets should stay in place (not moved)
    assert!(
        repo.file_exists("docs/diagrams/shared-logo.png"),
        "Shared asset should stay in docs/"
    );
    assert!(
        repo.file_exists("docs/images/banner.jpg"),
        "Shared asset should stay in docs/"
    );
}

#[test]
fn test_archive_fails_with_relative_shared_links() {
    let repo = TestRepo::new();
    repo.init_jit();

    // Configure archive categories
    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
"#,
    );
    repo.commit("Add config");

    // Create shared assets outside managed paths
    repo.write_file("shared/common-diagram.png", "shared diagram");

    // Create document with relative link to shared asset (will break when archived)
    repo.write_file(
        "dev/active/feature-z-design.md",
        r#"# Feature Z Design

See diagram: ![Diagram](../../shared/common-diagram.png)
"#,
    );
    repo.commit("Add feature Z with relative shared link");

    // Archive the document - should FAIL because relative link would break
    let output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/active/feature-z-design.md",
        "--type",
        "design",
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should fail
    assert!(
        !output.status.success(),
        "Archive should fail with relative shared links\nstderr: {}",
        stderr
    );

    // Document should NOT be archived (stayed in place)
    assert!(
        repo.file_exists("dev/active/feature-z-design.md"),
        "Document should remain in dev/active/ after failed archive"
    );
    assert!(
        !repo.file_exists("dev/archive/features/feature-z-design.md"),
        "Document should not be in archive after failed validation"
    );

    // Error message should mention the link issue
    assert!(
        stderr.contains("link") || stderr.contains("relative") || stderr.contains("break"),
        "Error should explain the link problem\nstderr: {}",
        stderr
    );
}

#[test]
fn test_atomic_operation_on_permission_error() {
    let repo = TestRepo::new();
    repo.init_jit();

    // Configure archive categories
    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
"#,
    );
    repo.commit("Add config");

    // Create document with multiple assets
    repo.write_file(
        "dev/active/multi-asset-doc.md",
        r#"# Multi Asset Doc

![Asset1](assets/asset1.png)
![Asset2](assets/asset2.png)
![Asset3](assets/asset3.png)
"#,
    );
    repo.write_file("dev/active/assets/asset1.png", "asset 1");
    repo.write_file("dev/active/assets/asset2.png", "asset 2");
    repo.write_file("dev/active/assets/asset3.png", "asset 3");
    repo.commit("Add doc with multiple assets");

    // Pre-create archive destination directory and make it read-only to cause failure
    repo.write_file("dev/archive/features/.keep", "");
    repo.commit("Create archive dir");
    repo.make_readonly("dev/archive/features");

    // Attempt to archive - should fail due to permission error
    let output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/active/multi-asset-doc.md",
        "--type",
        "design",
    ]);

    // Cleanup: restore permissions so temp cleanup works
    repo.make_writable("dev/archive/features");

    // Should fail
    assert!(
        !output.status.success(),
        "Archive should fail when destination is read-only"
    );

    // CRITICAL: Source files should all still exist (atomic rollback)
    assert!(
        repo.file_exists("dev/active/multi-asset-doc.md"),
        "Source document should remain after failed archive"
    );
    assert!(
        repo.file_exists("dev/active/assets/asset1.png"),
        "Source asset1 should remain after failed archive"
    );
    assert!(
        repo.file_exists("dev/active/assets/asset2.png"),
        "Source asset2 should remain after failed archive"
    );
    assert!(
        repo.file_exists("dev/active/assets/asset3.png"),
        "Source asset3 should remain after failed archive"
    );

    // CRITICAL: No partial files in destination (all-or-nothing)
    assert!(
        !repo.file_exists("dev/archive/features/multi-asset-doc.md"),
        "No partial document in archive after failure"
    );
    assert!(
        !repo.file_exists("dev/archive/features/assets/asset1.png"),
        "No partial assets in archive after failure"
    );
}

#[test]
fn test_archive_with_nested_assets_preserves_structure() {
    // This test would have caught Issue #2: Inconsistent path handling
    let repo = TestRepo::new();
    repo.init_jit();

    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
"#,
    );
    repo.commit("Add config");

    // Create document with assets in nested subdirectories
    repo.write_file(
        "dev/active/complex-design.md",
        r#"# Complex Design

![Icon](assets/icons/logo.svg)
![Screenshot](assets/screenshots/main.png)
![Diagram](assets/diagrams/flow.png)
"#,
    );
    repo.write_file("dev/active/assets/icons/logo.svg", "svg data");
    repo.write_file("dev/active/assets/screenshots/main.png", "screenshot");
    repo.write_file("dev/active/assets/diagrams/flow.png", "diagram");
    repo.commit("Add complex design with nested assets");

    // Archive the document
    let output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/active/complex-design.md",
        "--type",
        "design",
    ]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Archive should succeed with nested assets\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    // Verify nested structure is preserved in archive
    assert!(
        repo.file_exists("dev/archive/features/complex-design.md"),
        "Document should be archived"
    );
    assert!(
        repo.file_exists("dev/archive/features/assets/icons/logo.svg"),
        "Nested asset icons/logo.svg should preserve structure"
    );
    assert!(
        repo.file_exists("dev/archive/features/assets/screenshots/main.png"),
        "Nested asset screenshots/main.png should preserve structure"
    );
    assert!(
        repo.file_exists("dev/archive/features/assets/diagrams/flow.png"),
        "Nested asset diagrams/flow.png should preserve structure"
    );

    // Verify sources are removed
    assert!(
        !repo.file_exists("dev/active/complex-design.md"),
        "Source document should be removed"
    );
    assert!(
        !repo.file_exists("dev/active/assets/icons/logo.svg"),
        "Source nested asset should be removed"
    );
}

#[test]
fn test_archive_with_name_collisions_in_assets() {
    // This test would have caught Issue #1: Name collision in temp directory
    // If temp files use only filename, "config.json" from different dirs would collide
    let repo = TestRepo::new();
    repo.init_jit();

    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
"#,
    );
    repo.commit("Add config");

    // Create document with assets that have SAME FILENAME in different subdirectories
    repo.write_file(
        "dev/active/api-design.md",
        r#"# API Design

Frontend config: ![Config](assets/frontend/config.json)
Backend config: ![Config](assets/backend/config.json)
Database config: ![Config](assets/database/config.json)
"#,
    );
    repo.write_file("dev/active/assets/frontend/config.json", "frontend config");
    repo.write_file("dev/active/assets/backend/config.json", "backend config");
    repo.write_file("dev/active/assets/database/config.json", "database config");
    repo.commit("Add API design with same-named assets");

    // Archive the document
    let output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/active/api-design.md",
        "--type",
        "design",
    ]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Archive should succeed even with name collisions\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    // CRITICAL: All three config.json files should exist in their respective directories
    assert!(
        repo.file_exists("dev/archive/features/assets/frontend/config.json"),
        "Frontend config.json should be archived to correct location"
    );
    assert!(
        repo.file_exists("dev/archive/features/assets/backend/config.json"),
        "Backend config.json should be archived to correct location"
    );
    assert!(
        repo.file_exists("dev/archive/features/assets/database/config.json"),
        "Database config.json should be archived to correct location"
    );

    // Verify each file has correct content (not overwritten)
    let frontend_content = std::fs::read_to_string(
        repo.path()
            .join("dev/archive/features/assets/frontend/config.json"),
    )
    .expect("Read frontend config");
    assert_eq!(
        frontend_content, "frontend config",
        "Frontend config content preserved"
    );

    let backend_content = std::fs::read_to_string(
        repo.path()
            .join("dev/archive/features/assets/backend/config.json"),
    )
    .expect("Read backend config");
    assert_eq!(
        backend_content, "backend config",
        "Backend config content preserved"
    );

    let database_content = std::fs::read_to_string(
        repo.path()
            .join("dev/archive/features/assets/database/config.json"),
    )
    .expect("Read database config");
    assert_eq!(
        database_content, "database config",
        "Database config content preserved"
    );
}

#[test]
fn test_archive_fails_when_linked_to_active_issue() {
    let repo = TestRepo::new();
    repo.init_jit();

    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
"#,
    );
    repo.commit("Add config");

    // Create a document
    repo.write_file(
        "dev/active/important-design.md",
        "# Important Design\n\nThis is critical work.",
    );
    repo.commit("Add important design");

    // Create an issue
    let create_output =
        repo.run_jit(&["issue", "create", "--title", "Implement important feature"]);
    assert!(
        create_output.status.success(),
        "Issue creation should succeed"
    );

    // Extract issue ID from output
    let stdout = String::from_utf8_lossy(&create_output.stdout);
    let issue_id = stdout
        .lines()
        .find(|line| line.contains("Created issue"))
        .and_then(|line| line.split_whitespace().nth(2))
        .expect("Extract issue ID");

    // Transition to in-progress state
    let update_output = repo.run_jit(&["issue", "update", issue_id, "--state", "InProgress"]);
    assert!(
        update_output.status.success(),
        "Issue update to InProgress should succeed"
    );

    // Link the document to the active issue
    let link_output = repo.run_jit(&[
        "doc",
        "add",
        issue_id,
        "dev/active/important-design.md",
        "--label",
        "Design",
    ]);
    assert!(
        link_output.status.success(),
        "Document linking should succeed"
    );

    // Attempt to archive WITHOUT --force - should FAIL
    let archive_output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/active/important-design.md",
        "--type",
        "design",
    ]);

    let stderr = String::from_utf8_lossy(&archive_output.stderr);

    assert!(
        !archive_output.status.success(),
        "Archive should fail when linked to active issue\nstderr: {}",
        stderr
    );

    // Document should remain in place
    assert!(
        repo.file_exists("dev/active/important-design.md"),
        "Document should not be archived"
    );
    assert!(
        !repo.file_exists("dev/archive/features/important-design.md"),
        "Document should not be in archive"
    );

    // Error should mention active issue
    assert!(
        stderr.contains("active") || stderr.contains("in-progress") || stderr.contains("issue"),
        "Error should explain active issue problem\nstderr: {}",
        stderr
    );

    // Now attempt WITH --force - should SUCCEED
    let force_output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/active/important-design.md",
        "--type",
        "design",
        "--force",
    ]);

    let force_stdout = String::from_utf8_lossy(&force_output.stdout);
    let force_stderr = String::from_utf8_lossy(&force_output.stderr);

    assert!(
        force_output.status.success(),
        "Archive with --force should succeed\nstdout: {}\nstderr: {}",
        force_stdout,
        force_stderr
    );

    // Document should now be archived
    assert!(
        repo.file_exists("dev/archive/features/important-design.md"),
        "Document should be archived with --force"
    );
    assert!(
        !repo.file_exists("dev/active/important-design.md"),
        "Source document should be removed with --force"
    );
}

#[test]
fn test_archive_fails_for_permanent_docs() {
    let repo = TestRepo::new();
    repo.init_jit();

    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/", "dev/architecture/"]

[documentation.categories]
design = "features"
"#,
    );
    repo.commit("Add config");

    // Create permanent documents in protected paths
    repo.write_file(
        "docs/user-guide.md",
        "# User Guide\n\nPermanent documentation.",
    );
    repo.write_file(
        "dev/architecture/core-design.md",
        "# Core Architecture\n\nPermanent design.",
    );
    repo.commit("Add permanent docs");

    // Attempt to archive docs/user-guide.md
    let output1 = repo.run_jit(&["doc", "archive", "docs/user-guide.md", "--type", "design"]);

    let stderr1 = String::from_utf8_lossy(&output1.stderr);

    assert!(
        !output1.status.success(),
        "Archive should fail for docs/ path\nstderr: {}",
        stderr1
    );

    assert!(
        stderr1.contains("permanent") || stderr1.contains("protected") || stderr1.contains("docs/"),
        "Error should explain permanent path protection\nstderr: {}",
        stderr1
    );

    // Document should remain in place
    assert!(
        repo.file_exists("docs/user-guide.md"),
        "Permanent doc should not be archived"
    );

    // Attempt to archive dev/architecture/core-design.md
    let output2 = repo.run_jit(&[
        "doc",
        "archive",
        "dev/architecture/core-design.md",
        "--type",
        "design",
    ]);

    let stderr2 = String::from_utf8_lossy(&output2.stderr);

    assert!(
        !output2.status.success(),
        "Archive should fail for dev/architecture/ path\nstderr: {}",
        stderr2
    );

    assert!(
        repo.file_exists("dev/architecture/core-design.md"),
        "Architecture doc should not be archived"
    );
}

#[test]
fn test_dry_run_no_mutation() {
    let repo = TestRepo::new();
    repo.init_jit();

    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
"#,
    );
    repo.commit("Add config");

    // Create document with assets
    repo.write_file(
        "dev/active/test-design.md",
        r#"# Test Design

![Diagram](assets/diagram.png)
![Chart](assets/chart.svg)
"#,
    );
    repo.write_file("dev/active/assets/diagram.png", "diagram data");
    repo.write_file("dev/active/assets/chart.svg", "chart data");
    repo.commit("Add test design with assets");

    // Run archive with --dry-run
    let output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/active/test-design.md",
        "--type",
        "design",
        "--dry-run",
    ]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should succeed
    assert!(
        output.status.success(),
        "Dry-run should succeed\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    // Output should show the plan
    assert!(
        stdout.contains("dry-run") || stdout.contains("plan") || stdout.contains("would"),
        "Output should indicate dry-run mode\nstdout: {}",
        stdout
    );

    // CRITICAL: Source files should still exist (no mutation)
    assert!(
        repo.file_exists("dev/active/test-design.md"),
        "Source document should remain after dry-run"
    );
    assert!(
        repo.file_exists("dev/active/assets/diagram.png"),
        "Source asset diagram.png should remain after dry-run"
    );
    assert!(
        repo.file_exists("dev/active/assets/chart.svg"),
        "Source asset chart.svg should remain after dry-run"
    );

    // CRITICAL: Destination files should NOT exist
    assert!(
        !repo.file_exists("dev/archive/features/test-design.md"),
        "Destination document should not exist after dry-run"
    );
    assert!(
        !repo.file_exists("dev/archive/features/assets/diagram.png"),
        "Destination asset should not exist after dry-run"
    );
    assert!(
        !repo.file_exists("dev/archive/features/assets/chart.svg"),
        "Destination asset should not exist after dry-run"
    );

    // Verify no event was logged (check events.jsonl doesn't contain document_archived)
    let events_path = repo.path().join(".jit/data/events.jsonl");
    if events_path.exists() {
        let events = fs::read_to_string(&events_path).expect("Read events");
        assert!(
            !events.contains("document_archived"),
            "No document_archived event should be logged during dry-run"
        );
    }
}

#[test]
fn test_issue_metadata_updates_after_archive() {
    let repo = TestRepo::new();
    repo.init_jit();

    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
"#,
    );
    repo.commit("Add config");

    // Create a document
    repo.write_file(
        "dev/active/feature-design.md",
        "# Feature Design\n\nDetailed design for feature.",
    );
    repo.commit("Add feature design");

    // Create an issue
    let create_output = repo.run_jit(&["issue", "create", "--title", "Completed feature"]);
    assert!(
        create_output.status.success(),
        "Issue creation should succeed"
    );

    let stdout = String::from_utf8_lossy(&create_output.stdout);
    let issue_id = stdout
        .lines()
        .find(|line| line.contains("Created issue"))
        .and_then(|line| line.split_whitespace().nth(2))
        .expect("Extract issue ID");

    // Transition to done state (so archival is allowed)
    let update_output = repo.run_jit(&["issue", "update", issue_id, "--state", "Done"]);
    assert!(
        update_output.status.success(),
        "Issue update to Done should succeed"
    );

    // Link the document to the issue
    let link_output = repo.run_jit(&[
        "doc",
        "add",
        issue_id,
        "dev/active/feature-design.md",
        "--label",
        "Design",
    ]);
    assert!(
        link_output.status.success(),
        "Document linking should succeed"
    );

    // Verify initial document path in issue
    let show_before = repo.run_jit(&["issue", "show", issue_id, "--json"]);
    assert!(show_before.status.success(), "Issue show should succeed");
    let json_before = String::from_utf8_lossy(&show_before.stdout);
    assert!(
        json_before.contains("dev/active/feature-design.md"),
        "Issue should reference original path"
    );

    // Archive the document
    let archive_output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/active/feature-design.md",
        "--type",
        "design",
    ]);

    let archive_stdout = String::from_utf8_lossy(&archive_output.stdout);
    let archive_stderr = String::from_utf8_lossy(&archive_output.stderr);

    assert!(
        archive_output.status.success(),
        "Archive should succeed\nstdout: {}\nstderr: {}",
        archive_stdout,
        archive_stderr
    );

    // Load issue and verify DocumentReference.path updated to new location
    let show_after = repo.run_jit(&["issue", "show", issue_id, "--json"]);
    assert!(
        show_after.status.success(),
        "Issue show after archive should succeed"
    );
    let json_after = String::from_utf8_lossy(&show_after.stdout);

    // Should now reference archived path
    assert!(
        json_after.contains("dev/archive/features/feature-design.md"),
        "Issue should reference archived path\nJSON: {}",
        json_after
    );

    // Should NOT contain old path
    assert!(
        !json_after.contains("dev/active/feature-design.md"),
        "Issue should not reference old path\nJSON: {}",
        json_after
    );
}

#[test]
fn test_doc_show_works_post_archival() {
    let repo = TestRepo::new();
    repo.init_jit();

    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
session = "sessions"
"#,
    );
    repo.commit("Add config");

    // Create a document
    repo.write_file(
        "dev/sessions/session-42.md",
        "# Session 42\n\nSession notes for task completion.",
    );
    repo.commit("Add session notes");

    // Create an issue
    let create_output = repo.run_jit(&["issue", "create", "--title", "Complete task 42"]);
    assert!(
        create_output.status.success(),
        "Issue creation should succeed"
    );

    let stdout = String::from_utf8_lossy(&create_output.stdout);
    let issue_id = stdout
        .lines()
        .find(|line| line.contains("Created issue"))
        .and_then(|line| line.split_whitespace().nth(2))
        .expect("Extract issue ID");

    // Transition to done state (so archival is allowed)
    let update_output = repo.run_jit(&["issue", "update", issue_id, "--state", "done"]);
    assert!(
        update_output.status.success(),
        "Issue update to Done should succeed"
    );

    let link_output = repo.run_jit(&[
        "doc",
        "add",
        issue_id,
        "dev/sessions/session-42.md",
        "--label",
        "Session Notes",
    ]);
    assert!(
        link_output.status.success(),
        "Document linking should succeed"
    );

    // Verify doc show works before archival
    let show_before = repo.run_jit(&["doc", "show", issue_id, "dev/sessions/session-42.md"]);
    assert!(
        show_before.status.success(),
        "Doc show should work before archival"
    );
    let stdout_before = String::from_utf8_lossy(&show_before.stdout);
    assert!(
        stdout_before.contains("Session 42") || stdout_before.contains("session-42.md"),
        "Doc show should display document info"
    );

    // Archive the document
    let archive_output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/sessions/session-42.md",
        "--type",
        "session",
    ]);

    let archive_stdout = String::from_utf8_lossy(&archive_output.stdout);
    let archive_stderr = String::from_utf8_lossy(&archive_output.stderr);

    assert!(
        archive_output.status.success(),
        "Archive should succeed\nstdout: {}\nstderr: {}",
        archive_stdout,
        archive_stderr
    );

    // Commit the archived file to git (doc show reads from git)
    repo.commit("Archive session");

    // Verify doc show still works after archival
    let show_after = repo.run_jit(&[
        "doc",
        "show",
        issue_id,
        "dev/archive/sessions/session-42.md",
    ]);

    let stdout_after = String::from_utf8_lossy(&show_after.stdout);
    let stderr_after = String::from_utf8_lossy(&show_after.stderr);

    assert!(
        show_after.status.success(),
        "Doc show should work after archival\nstdout: {}\nstderr: {}",
        stdout_after,
        stderr_after
    );

    // Should display archived document content
    assert!(
        stdout_after.contains("Session 42"),
        "Doc show should display archived document content\nstdout: {}",
        stdout_after
    );
}

#[test]
fn test_archive_preserves_asset_references() {
    // Test that verifies archived document's asset references remain valid
    // This is what post-archival verification checks
    let repo = TestRepo::new();
    repo.init_jit();

    repo.write_file(
        ".jit/config.toml",
        r#"
[documentation]
development_root = "dev"
managed_paths = ["dev/active"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

[documentation.categories]
design = "features"
"#,
    );
    repo.commit("Add config");

    // Create document with nested per-doc assets
    repo.write_file(
        "dev/active/feature-design.md",
        r#"# Feature Design

![Icon](assets/icons/icon.svg)
![Screenshot](assets/screens/main.png)
"#,
    );
    repo.write_file("dev/active/assets/icons/icon.svg", "svg data");
    repo.write_file("dev/active/assets/screens/main.png", "png data");
    repo.commit("Add design with nested assets");

    // Archive the document
    let output = repo.run_jit(&[
        "doc",
        "archive",
        "dev/active/feature-design.md",
        "--type",
        "design",
    ]);

    assert!(
        output.status.success(),
        "Archive should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify all assets exist at their new locations
    // Post-archival verification should have checked this
    assert!(
        repo.file_exists("dev/archive/features/feature-design.md"),
        "Document should be archived"
    );
    assert!(
        repo.file_exists("dev/archive/features/assets/icons/icon.svg"),
        "Nested asset icon.svg should be accessible"
    );
    assert!(
        repo.file_exists("dev/archive/features/assets/screens/main.png"),
        "Nested asset main.png should be accessible"
    );

    // Verify document content still references correct relative paths
    let archived_content =
        std::fs::read_to_string(repo.path().join("dev/archive/features/feature-design.md"))
            .expect("Read archived document");

    assert!(
        archived_content.contains("assets/icons/icon.svg"),
        "Document should still reference assets with relative paths"
    );
    assert!(
        archived_content.contains("assets/screens/main.png"),
        "Document should still reference assets with relative paths"
    );
}
