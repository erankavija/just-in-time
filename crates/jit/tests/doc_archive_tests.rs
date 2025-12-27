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
