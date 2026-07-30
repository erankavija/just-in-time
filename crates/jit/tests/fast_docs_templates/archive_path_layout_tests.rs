use std::fs;
use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn visit_files(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root)
        .unwrap()
        .flat_map(|entry| {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit_files(&path)
            } else {
                vec![path]
            }
        })
        .collect()
}

fn repeats_container_owner(archive_root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(archive_root).unwrap();
    let components = relative.components().collect::<Vec<_>>();
    matches!(components.as_slice(), [container, _area, repeated, ..] if container == repeated)
}

#[test]
fn test_repeated_container_owner_detection_ignores_unrelated_vendor_repetition() {
    let archive_root = Path::new("archive");

    assert!(repeats_container_owner(
        archive_root,
        Path::new("archive/abcdef12-platform/active/abcdef12-platform/plan.md"),
    ));
    assert!(repeats_container_owner(
        archive_root,
        Path::new("archive/abcdef12-platform/presentations/abcdef12-platform/talk.html"),
    ));
    assert!(!repeats_container_owner(
        archive_root,
        Path::new("archive/abcdef12-platform/presentations/vendor/reveal.js/reveal.js"),
    ));
}

#[test]
fn test_repository_archive_paths_do_not_repeat_policy_roots_inside_container_directories() {
    let archive_root = repository_root().join("dev/archive");
    let offenders = visit_files(&archive_root)
        .into_iter()
        .filter(|path| {
            let relative = path.strip_prefix(&archive_root).unwrap();
            let mut components = relative.components();
            let container = components.next().unwrap();
            let policy_root_repeated = components.next().is_some_and(|component| {
                component.as_os_str() == std::ffi::OsStr::new("dev")
                    || component.as_os_str() == std::ffi::OsStr::new("archive")
            });
            policy_root_repeated
                || repeats_container_owner(&archive_root, path)
                || container.as_os_str() == std::ffi::OsStr::new("dev")
                || container.as_os_str() == std::ffi::OsStr::new("archive")
        })
        .map(|path| {
            path.strip_prefix(repository_root())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();

    assert!(
        offenders.is_empty(),
        "container archives must store development-root-relative artifacts, name container ownership once, and must not archive the archive root itself: {offenders:?}"
    );
}
