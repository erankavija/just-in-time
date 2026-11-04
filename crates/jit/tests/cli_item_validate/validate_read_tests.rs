//! `jit validate` as a lock-free read of the repository.

use jit::test_cli::JitCli;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Every file under `root` but Git's, with its bytes.
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, current: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(current).unwrap() {
            let path = entry.unwrap().path();
            match path.is_dir() {
                true if path.ends_with(".git") => {}
                true => visit(root, &path, files),
                false => {
                    files.insert(
                        path.strip_prefix(root).unwrap().to_path_buf(),
                        fs::read(&path).unwrap(),
                    );
                }
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

/// A fresh clone holds none of the machine-local lock files, and validating it
/// creates none.
#[test]
fn test_validate_leaves_no_file_behind_in_a_repository_without_lock_files() {
    let cli = JitCli::initialised().unwrap();
    cli.create_issue(["Validated"]).unwrap();
    files(cli.root())
        .into_keys()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "lock")
        })
        .for_each(|lock| fs::remove_file(cli.root().join(lock)).unwrap());
    let before = files(cli.root());

    let report = cli.run_json(["validate"]).unwrap();

    let after = files(cli.root());
    assert_eq!(report["valid"], true, "{report}");
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>()
    );
    assert!(after == before, "validation rewrote a file");
}
