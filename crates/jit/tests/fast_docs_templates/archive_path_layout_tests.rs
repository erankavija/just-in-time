use jit::config::JitConfig;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

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

fn repeats_container_owner(
    archive_root: &Path,
    development_root: &Path,
    issue_scoped_areas: &[String],
    path: &Path,
) -> bool {
    let relative = path.strip_prefix(archive_root).unwrap();
    let components = relative.components().collect::<Vec<_>>();
    let Some((container, beneath_container)) = components.split_first() else {
        return false;
    };
    issue_scoped_areas.iter().any(|area| {
        let Ok(relative_area) = Path::new(area).strip_prefix(development_root) else {
            return false;
        };
        let area = relative_area.components().collect::<Vec<_>>();
        beneath_container.starts_with(&area)
            && beneath_container.get(area.len()) == Some(container)
            && beneath_container.len() > area.len() + 1
    })
}

fn archive_tree_offenders(repository_root: &Path, config: &JitConfig) -> Vec<String> {
    let documentation = config.documentation.as_ref().unwrap();
    let development_root = documentation.development_root();
    let issue_scoped_areas = documentation.issue_scoped_areas();
    let archive_root = repository_root.join(documentation.archive_root());
    let mut offenders = visit_files(&archive_root)
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
                || repeats_container_owner(
                    &archive_root,
                    Path::new(&development_root),
                    &issue_scoped_areas,
                    path,
                )
                || container.as_os_str() == std::ffi::OsStr::new("dev")
                || container.as_os_str() == std::ffi::OsStr::new("archive")
        })
        .map(|path| {
            path.strip_prefix(repository_root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    offenders.sort();
    offenders
}

#[test]
fn test_repeated_container_owner_detection_respects_configured_areas_and_vendor_repetition() {
    let archive_root = Path::new("archive");
    let development_root = Path::new("dev");
    let issue_scoped_areas = vec!["dev/active".to_string(), "dev/team/slides".to_string()];

    assert!(repeats_container_owner(
        archive_root,
        development_root,
        &issue_scoped_areas,
        Path::new("archive/abcdef12-platform/active/abcdef12-platform/plan.md"),
    ));
    assert!(repeats_container_owner(
        archive_root,
        development_root,
        &issue_scoped_areas,
        Path::new("archive/abcdef12-platform/team/slides/abcdef12-platform/talk.html"),
    ));
    assert!(!repeats_container_owner(
        archive_root,
        development_root,
        &issue_scoped_areas,
        Path::new("archive/abcdef12-platform/generated/abcdef12-platform/report.md"),
    ));
    assert!(!repeats_container_owner(
        archive_root,
        development_root,
        &issue_scoped_areas,
        Path::new("archive/abcdef12-platform/team/abcdef12-platform/notes.md"),
    ));
    assert!(!repeats_container_owner(
        archive_root,
        development_root,
        &issue_scoped_areas,
        Path::new("archive/abcdef12-platform/presentations/vendor/reveal.js/reveal.js"),
    ));
}

#[test]
fn test_archive_tree_offenders_name_only_repeated_policy_roots_and_container_owners() {
    let root = TempDir::new().unwrap();
    let config: JitConfig = toml::from_str(
        "[documentation]\ndevelopment_root = \"dev\"\narchive_root = \"dev/archive\"\n\
         issue_scoped_areas = [\"dev/active\"]\n",
    )
    .unwrap();
    let write = |relative: &str| {
        let path = root.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    };
    let offending = [
        "dev/archive/abcdef12-platform/dev/active/plan.md",
        "dev/archive/abcdef12-platform/archive/plan.md",
        "dev/archive/abcdef12-platform/active/abcdef12-platform/plan.md",
        "dev/archive/dev/plan.md",
        "dev/archive/archive/plan.md",
    ];
    offending.iter().for_each(|relative| write(relative));
    write("dev/archive/abcdef12-platform/active/plan.md");
    write("dev/archive/abcdef12-platform/notes/abcdef12-platform/plan.md");

    let mut expected = offending.map(str::to_string).to_vec();
    expected.sort();
    assert_eq!(archive_tree_offenders(root.path(), &config), expected);
}
