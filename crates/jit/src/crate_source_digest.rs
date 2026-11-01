//! The `jit` crate's source digest, computed by `build.rs` to key the fixture tests.

use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Crate-relative paths whose files make up the crate source.
pub const CRATE_SOURCE_ROOTS: [&str; 3] = ["Cargo.toml", "build.rs", "src"];

/// SHA-256 over every file under `roots`, framed by crate-relative path in path order.
pub fn crate_source_sha256(crate_root: &Path, roots: &[&str]) -> io::Result<String> {
    let mut files = roots
        .iter()
        .map(|root| source_files(&crate_root.join(root)))
        .collect::<io::Result<Vec<_>>>()?
        .concat();
    files.sort();
    files
        .iter()
        .try_fold(Sha256::new(), |mut hasher, path| {
            let relative = path
                .strip_prefix(crate_root)
                .map_err(|error| io::Error::other(error.to_string()))?
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let bytes = fs::read(path)?;
            [relative.as_bytes(), bytes.as_slice()]
                .iter()
                .for_each(|field| {
                    hasher.update((field.len() as u64).to_le_bytes());
                    hasher.update(field);
                });
            Ok(hasher)
        })
        .map(|hasher| format!("{:x}", hasher.finalize()))
}

fn source_files(path: &Path) -> io::Result<Vec<PathBuf>> {
    if !fs::metadata(path)?.is_dir() {
        return Ok(vec![path.to_path_buf()]);
    }
    fs::read_dir(path)?
        .map(|entry| source_files(&entry?.path()))
        .collect::<io::Result<Vec<_>>>()
        .map(|nested| nested.concat())
}
