use jit::domain::artifact_plan::ContentIdentity;
use jit::errors::AlreadyExistsError;
use jit::storage::{ArtifactMutationError, JsonFileStorage};
use std::fs;
use tempfile::TempDir;

fn storage() -> (TempDir, JsonFileStorage) {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".jit")).unwrap();
    let storage = JsonFileStorage::new(repo.path().join(".jit"));
    (repo, storage)
}

#[test]
fn test_physical_repo_path_rejects_symlink_leaf_and_traversal() {
    let (repo, storage) = storage();
    fs::create_dir_all(repo.path().join("docs/real")).unwrap();
    fs::write(repo.path().join("docs/real/file.md"), b"safe").unwrap();

    assert_eq!(
        storage.physical_repo_path("docs/real/file.md").unwrap(),
        repo.path().join("docs/real/file.md")
    );

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("real", repo.path().join("docs/linked-dir")).unwrap();
        std::os::unix::fs::symlink("real/file.md", repo.path().join("docs/linked-file.md"))
            .unwrap();

        for path in ["docs/linked-dir/file.md", "docs/linked-file.md"] {
            let error = storage.physical_repo_path(path).unwrap_err();
            assert!(matches!(
                error.downcast_ref::<ArtifactMutationError>(),
                Some(ArtifactMutationError::SymlinkArtifact { .. })
            ));
        }
    }

    assert!(storage.physical_repo_path("../outside.md").is_err());
}

#[test]
fn test_stage_and_verify_use_caller_supplied_identity() {
    let (repo, storage) = storage();
    fs::create_dir(repo.path().join("docs")).unwrap();
    fs::write(repo.path().join("docs/source.bin"), b"recorded bytes").unwrap();

    let recorded = ContentIdentity::from_bytes(b"recorded bytes");
    let staged = storage.stage_artifact("docs/source.bin").unwrap();
    storage.verify_staged_artifact(staged, &recorded).unwrap();

    let wrong = ContentIdentity::from_bytes(b"different bytes");
    let staged = storage.stage_artifact("docs/source.bin").unwrap();
    let error = storage.verify_staged_artifact(staged, &wrong).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ArtifactMutationError>(),
        Some(ArtifactMutationError::IdentityMismatch { .. })
    ));
}

#[test]
fn test_publish_staged_artifact_atomically_creates_destination() {
    let (repo, storage) = storage();
    fs::create_dir(repo.path().join("docs")).unwrap();
    fs::write(repo.path().join("docs/source.md"), b"publish me").unwrap();

    let verified = storage
        .verify_staged_artifact(
            storage.stage_artifact("docs/source.md").unwrap(),
            &ContentIdentity::from_bytes(b"publish me"),
        )
        .unwrap();
    storage
        .publish_staged_artifact(verified, "archive/nested/source.md")
        .unwrap();

    assert_eq!(
        fs::read(repo.path().join("archive/nested/source.md")).unwrap(),
        b"publish me"
    );
    assert_eq!(
        fs::read(repo.path().join("docs/source.md")).unwrap(),
        b"publish me",
        "publication does not decide whether the source is deletion-eligible"
    );
}

#[test]
fn test_publish_rejects_every_occupied_destination_and_preserves_bytes() {
    for occupied_bytes in [b"foreign bytes".as_slice(), b"staged bytes".as_slice()] {
        let (repo, storage) = storage();
        fs::create_dir_all(repo.path().join("docs")).unwrap();
        fs::create_dir_all(repo.path().join("archive")).unwrap();
        fs::write(repo.path().join("docs/source.md"), b"staged bytes").unwrap();
        fs::write(repo.path().join("archive/source.md"), occupied_bytes).unwrap();

        let verified = storage
            .verify_staged_artifact(
                storage.stage_artifact("docs/source.md").unwrap(),
                &ContentIdentity::from_bytes(b"staged bytes"),
            )
            .unwrap();
        let error = storage
            .publish_staged_artifact(verified, "archive/source.md")
            .unwrap_err();

        assert!(error.downcast_ref::<AlreadyExistsError>().is_some());
        assert_eq!(
            fs::read(repo.path().join("archive/source.md")).unwrap(),
            occupied_bytes
        );
    }
}

#[test]
fn test_publish_loses_race_to_external_destination_without_overwrite() {
    let (repo, storage) = storage();
    fs::create_dir(repo.path().join("docs")).unwrap();
    fs::create_dir(repo.path().join("archive")).unwrap();
    fs::write(repo.path().join("docs/source.md"), b"planned bytes").unwrap();

    // Planning/staging observed a free destination. An external writer then
    // occupies it before finalization; publish must not use check-then-rename.
    let verified = storage
        .verify_staged_artifact(
            storage.stage_artifact("docs/source.md").unwrap(),
            &ContentIdentity::from_bytes(b"planned bytes"),
        )
        .unwrap();
    fs::write(repo.path().join("archive/source.md"), b"race winner").unwrap();

    let error = storage
        .publish_staged_artifact(verified, "archive/source.md")
        .unwrap_err();
    assert!(error.downcast_ref::<AlreadyExistsError>().is_some());
    assert_eq!(
        fs::read(repo.path().join("archive/source.md")).unwrap(),
        b"race winner"
    );
}

#[cfg(unix)]
#[test]
fn test_publish_classifies_occupied_symlink_leaf_as_already_exists() {
    let (repo, storage) = storage();
    fs::create_dir_all(repo.path().join("docs")).unwrap();
    fs::create_dir_all(repo.path().join("archive")).unwrap();
    fs::write(repo.path().join("docs/source.md"), b"staged bytes").unwrap();
    fs::write(repo.path().join("archive/target.md"), b"target bytes").unwrap();
    std::os::unix::fs::symlink("target.md", repo.path().join("archive/source.md")).unwrap();

    let verified = storage
        .verify_staged_artifact(
            storage.stage_artifact("docs/source.md").unwrap(),
            &ContentIdentity::from_bytes(b"staged bytes"),
        )
        .unwrap();
    let error = storage
        .publish_staged_artifact(verified, "archive/source.md")
        .unwrap_err();

    assert!(error.downcast_ref::<AlreadyExistsError>().is_some());
    assert_eq!(
        fs::read(repo.path().join("archive/target.md")).unwrap(),
        b"target bytes"
    );
    assert!(fs::symlink_metadata(repo.path().join("archive/source.md"))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[cfg(unix)]
#[test]
fn test_publish_rejects_symlinked_parent_before_destination_leaf() {
    let (repo, storage) = storage();
    fs::create_dir_all(repo.path().join("docs")).unwrap();
    fs::create_dir_all(repo.path().join("real-archive")).unwrap();
    fs::write(repo.path().join("docs/source.md"), b"staged bytes").unwrap();
    std::os::unix::fs::symlink("real-archive", repo.path().join("archive")).unwrap();

    let verified = storage
        .verify_staged_artifact(
            storage.stage_artifact("docs/source.md").unwrap(),
            &ContentIdentity::from_bytes(b"staged bytes"),
        )
        .unwrap();
    let error = storage
        .publish_staged_artifact(verified, "archive/source.md")
        .unwrap_err();

    assert!(matches!(
        error.downcast_ref::<ArtifactMutationError>(),
        Some(ArtifactMutationError::SymlinkArtifact { .. })
    ));
    assert!(!repo.path().join("real-archive/source.md").exists());
}

#[test]
fn test_delete_artifact_requires_matching_recorded_identity() {
    let (repo, storage) = storage();
    fs::create_dir(repo.path().join("docs")).unwrap();
    let source = repo.path().join("docs/source.md");
    fs::write(&source, b"edited after planning").unwrap();

    let error = storage
        .delete_artifact_if_identity(
            "docs/source.md",
            &ContentIdentity::from_bytes(b"planned bytes"),
        )
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ArtifactMutationError>(),
        Some(ArtifactMutationError::IdentityMismatch { .. })
    ));
    assert_eq!(fs::read(&source).unwrap(), b"edited after planning");

    storage
        .delete_artifact_if_identity(
            "docs/source.md",
            &ContentIdentity::from_bytes(b"edited after planning"),
        )
        .unwrap();
    assert!(!source.exists());
}

#[test]
fn test_cross_filesystem_error_names_staged_and_destination_paths() {
    let error = ArtifactMutationError::CrossFilesystem {
        staged_path: "/repo/.jit/tmp/staged".into(),
        destination_path: "/repo/archive/file.md".into(),
    };
    let message = error.to_string();

    assert!(message.contains("/repo/.jit/tmp/staged"));
    assert!(message.contains("/repo/archive/file.md"));
}
