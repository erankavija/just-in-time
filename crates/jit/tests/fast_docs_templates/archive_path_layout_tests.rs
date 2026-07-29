use std::fs;
use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn test_repository_archive_paths_do_not_repeat_policy_roots_inside_container_directories() {
    let archive_root = repository_root().join("dev/archive");
    let offenders = fs::read_dir(&archive_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .flat_map(|container| {
            ["dev", "archive"]
                .into_iter()
                .map(move |root| container.join(root))
        })
        .filter(|path| path.exists())
        .map(|path| {
            path.strip_prefix(repository_root())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();

    assert!(
        offenders.is_empty(),
        "container archives must store development-root-relative artifacts and must not archive the archive root itself: {offenders:?}"
    );
}
