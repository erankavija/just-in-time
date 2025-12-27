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
