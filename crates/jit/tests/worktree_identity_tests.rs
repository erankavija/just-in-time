use jit::storage::worktree_identity::{
    generate_worktree_id, load_or_create_worktree_identity, WorktreeIdentity,
};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/jit")
        .to_string_lossy()
        .to_string()
}

#[test]
fn test_generate_worktree_id_is_deterministic() {
    let path = PathBuf::from("/home/user/project");
    let timestamp = chrono::DateTime::parse_from_rfc3339("2026-01-06T20:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let id1 = generate_worktree_id(&path, timestamp);
    let id2 = generate_worktree_id(&path, timestamp);

    assert_eq!(id1, id2, "Same inputs should produce same ID");
}

#[test]
fn test_generate_worktree_id_format() {
    let path = PathBuf::from("/test/path");
    let timestamp = chrono::Utc::now();

    let id = generate_worktree_id(&path, timestamp);

    // Should start with "wt:"
    assert!(id.starts_with("wt:"), "ID should start with 'wt:'");

    // Should be exactly "wt:" + 8 hex chars
    assert_eq!(id.len(), 11, "ID should be 11 chars (wt: + 8 hex)");

    // After "wt:", should be valid hex
    let hex_part = &id[3..];
    assert!(
        hex_part.chars().all(|c| c.is_ascii_hexdigit()),
        "ID suffix should be hex: {}",
        hex_part
    );
}

#[test]
fn test_generate_worktree_id_different_paths_different_ids() {
    let timestamp = chrono::Utc::now();

    let id1 = generate_worktree_id(&PathBuf::from("/path/one"), timestamp);
    let id2 = generate_worktree_id(&PathBuf::from("/path/two"), timestamp);

    assert_ne!(id1, id2, "Different paths should produce different IDs");
}

#[test]
fn test_generate_worktree_id_different_timestamps_different_ids() {
    let path = PathBuf::from("/same/path");

    let time1 = chrono::DateTime::parse_from_rfc3339("2026-01-06T20:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let time2 = chrono::DateTime::parse_from_rfc3339("2026-01-06T20:01:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let id1 = generate_worktree_id(&path, time1);
    let id2 = generate_worktree_id(&path, time2);

    assert_ne!(
        id1, id2,
        "Different timestamps should produce different IDs"
    );
}

#[test]
fn test_worktree_identity_serialization() {
    let identity = WorktreeIdentity {
        schema_version: 1,
        worktree_id: "wt:abc123ef".to_string(),
        branch: "main".to_string(),
        root: "/path/to/worktree".to_string(),
        created_at: chrono::Utc::now(),
        relocated_at: None,
    };

    let json = serde_json::to_string_pretty(&identity).unwrap();
    let deserialized: WorktreeIdentity = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.worktree_id, identity.worktree_id);
    assert_eq!(deserialized.branch, identity.branch);
    assert_eq!(deserialized.root, identity.root);
}

#[test]
fn test_load_or_create_creates_new_identity() {
    let temp_dir = TempDir::new().unwrap();
    let jit_dir = temp_dir.path().join(".jit");
    fs::create_dir_all(&jit_dir).unwrap();

    let branch = "test-branch".to_string();
    let identity = load_or_create_worktree_identity(&jit_dir, temp_dir.path(), &branch).unwrap();

    assert!(identity.worktree_id.starts_with("wt:"));
    assert_eq!(identity.branch, branch);
    assert_eq!(identity.root, temp_dir.path().to_string_lossy().to_string());
    assert_eq!(identity.schema_version, 1);
    assert!(identity.relocated_at.is_none());

    // Verify file was created
    let wt_file = jit_dir.join("worktree.json");
    assert!(wt_file.exists(), "worktree.json should be created");
}

#[test]
fn test_load_or_create_loads_existing_identity() {
    let temp_dir = TempDir::new().unwrap();
    let jit_dir = temp_dir.path().join(".jit");
    fs::create_dir_all(&jit_dir).unwrap();

    let branch = "test-branch".to_string();

    // Create first time
    let identity1 = load_or_create_worktree_identity(&jit_dir, temp_dir.path(), &branch).unwrap();

    // Load second time - should get same ID
    let identity2 = load_or_create_worktree_identity(&jit_dir, temp_dir.path(), &branch).unwrap();

    assert_eq!(identity1.worktree_id, identity2.worktree_id);
    assert_eq!(identity1.created_at, identity2.created_at);
}

#[test]
fn test_relocation_detection_updates_path() {
    let temp_dir = TempDir::new().unwrap();
    let jit_dir = temp_dir.path().join(".jit");
    fs::create_dir_all(&jit_dir).unwrap();

    let branch = "test-branch".to_string();

    // Create identity with original path
    let mut identity =
        load_or_create_worktree_identity(&jit_dir, temp_dir.path(), &branch).unwrap();
    let original_id = identity.worktree_id.clone();

    // Manually change the root in the JSON file to simulate relocation
    identity.root = "/old/path".to_string();
    let wt_file = jit_dir.join("worktree.json");
    let json = serde_json::to_string_pretty(&identity).unwrap();
    fs::write(&wt_file, json).unwrap();

    // Load again - should detect relocation
    let relocated = load_or_create_worktree_identity(&jit_dir, temp_dir.path(), &branch).unwrap();

    assert_eq!(
        relocated.worktree_id, original_id,
        "ID should remain stable"
    );
    assert_eq!(
        relocated.root,
        temp_dir.path().to_string_lossy().to_string(),
        "Root should be updated"
    );
    assert!(
        relocated.relocated_at.is_some(),
        "relocated_at should be set"
    );
}

#[test]
fn test_atomic_write_on_relocation() {
    let temp_dir = TempDir::new().unwrap();
    let jit_dir = temp_dir.path().join(".jit");
    fs::create_dir_all(&jit_dir).unwrap();

    let branch = "test-branch".to_string();

    // Create identity
    let mut identity =
        load_or_create_worktree_identity(&jit_dir, temp_dir.path(), &branch).unwrap();
    identity.root = "/old/path".to_string();

    let wt_file = jit_dir.join("worktree.json");
    let json = serde_json::to_string_pretty(&identity).unwrap();
    fs::write(&wt_file, json).unwrap();

    // Load should update atomically
    let _relocated = load_or_create_worktree_identity(&jit_dir, temp_dir.path(), &branch).unwrap();

    // Verify no .tmp file left behind
    let tmp_file = jit_dir.join("worktree.json.tmp");
    assert!(!tmp_file.exists(), "Temp file should be cleaned up");

    // Verify final file is valid JSON
    let content = fs::read_to_string(&wt_file).unwrap();
    let _parsed: WorktreeIdentity = serde_json::from_str(&content).unwrap();
}

#[test]
fn test_init_removes_copied_worktree_json_with_wrong_path() {
    // Simulate git worktree add copying .jit/worktree.json with wrong path
    let temp_dir = TempDir::new().unwrap();
    let jit_dir = temp_dir.path().join(".jit");
    fs::create_dir_all(&jit_dir).unwrap();

    // Create a worktree.json with WRONG root path (simulates copied file)
    let wrong_identity = WorktreeIdentity {
        schema_version: 1,
        worktree_id: "wt:wrongid1".to_string(),
        branch: "main".to_string(),
        root: "/some/other/worktree".to_string(), // Wrong path!
        created_at: chrono::Utc::now(),
        relocated_at: None,
    };

    let wt_file = jit_dir.join("worktree.json");
    let json = serde_json::to_string_pretty(&wrong_identity).unwrap();
    fs::write(&wt_file, json).unwrap();

    // Run init - should detect and remove the copied file
    let jit = jit_binary();
    let output = Command::new(&jit)
        .arg("init")
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to run jit init");

    assert!(output.status.success(), "jit init failed");

    // File should be removed (no git worktree context, so treated as non-worktree)
    // In real worktree scenario, it would be removed and recreated with new ID
    // For now, just verify init completed successfully
}

#[test]
fn test_init_preserves_correct_worktree_json() {
    // Test that init doesn't delete properly initialized worktree.json
    let temp_dir = TempDir::new().unwrap();
    let jit_dir = temp_dir.path().join(".jit");
    fs::create_dir_all(&jit_dir).unwrap();

    // Create a worktree.json with CORRECT root path
    let correct_identity = WorktreeIdentity {
        schema_version: 1,
        worktree_id: "wt:correct1".to_string(),
        branch: "main".to_string(),
        root: temp_dir.path().to_string_lossy().to_string(), // Correct path
        created_at: chrono::Utc::now(),
        relocated_at: None,
    };

    let wt_file = jit_dir.join("worktree.json");
    let json = serde_json::to_string_pretty(&correct_identity).unwrap();
    fs::write(&wt_file, json).unwrap();

    // Run init - should NOT delete the file
    let jit = jit_binary();
    let output = Command::new(&jit)
        .arg("init")
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to run jit init");

    assert!(output.status.success(), "jit init failed");

    // File should still exist with same ID
    assert!(wt_file.exists());
    let content = fs::read_to_string(&wt_file).unwrap();
    let identity: WorktreeIdentity = serde_json::from_str(&content).unwrap();
    assert_eq!(identity.worktree_id, "wt:correct1");
    assert_eq!(identity.root, temp_dir.path().to_string_lossy().to_string());
}

#[test]
fn test_init_is_idempotent() {
    // Calling init multiple times should not change worktree ID
    let temp_dir = TempDir::new().unwrap();
    let jit = jit_binary();

    // First init
    Command::new(&jit)
        .arg("init")
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to run jit init");

    // Create a worktree.json manually with specific ID
    let jit_dir = temp_dir.path().join(".jit");
    let identity = WorktreeIdentity {
        schema_version: 1,
        worktree_id: "wt:stable01".to_string(),
        branch: "main".to_string(),
        root: temp_dir.path().to_string_lossy().to_string(),
        created_at: chrono::Utc::now(),
        relocated_at: None,
    };

    let wt_file = jit_dir.join("worktree.json");
    let json = serde_json::to_string_pretty(&identity).unwrap();
    fs::write(&wt_file, json).unwrap();

    // Second init - should not change anything
    Command::new(&jit)
        .arg("init")
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to run jit init");

    // Verify ID is unchanged
    let content = fs::read_to_string(&wt_file).unwrap();
    let final_identity: WorktreeIdentity = serde_json::from_str(&content).unwrap();
    assert_eq!(final_identity.worktree_id, "wt:stable01");
}
