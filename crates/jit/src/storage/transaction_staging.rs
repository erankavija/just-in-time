//! Capability-relative synchronized stage creation.

use cap_std::fs::{Dir, OpenOptions};
use std::io::{self, Write};

pub(crate) fn stage_bytes(directory: &Dir, name: &str, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = directory.open_with(name, &options)?;
    file.write_all(bytes)?;
    file.sync_all()
}

pub(crate) fn sync_directory(directory: &Dir) -> io::Result<()> {
    directory.try_clone()?.into_std_file().sync_all()
}
