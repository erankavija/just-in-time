//! Assemble this repository's workflow package tree at a destination.
//!
//! Invoke it through `scripts/assemble-package.sh`, which is the entry point
//! the assembly's own documentation names. The render lives in
//! `jit::profile::package_assembly`, which draws each declared source from the
//! side that owns it — a live asset from the repository file its declaration
//! targets, everything else from the checked-in package sources — and publishes
//! a freshly staged tree; this file supplies only the I/O around it.
//!
//! The package sources come from the checkout cargo compiles this example out
//! of, so the tree produced is the one this working tree declares, and the
//! destination comes from the caller, because the consumer of the tree is a
//! release job staging its own directory.
//!
//! Usage:
//!   assemble-package <destination>
//!
//! Exit codes:
//!   0 — the tree is assembled at the destination
//!   1 — the assembly or the publication failed
//!   2 — a usage error

use jit::profile::package_assembly::{assemble_package_tree, PACKAGE_SOURCE_PATH};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Exit code for a usage error, matching the entry-point scripts' convention.
const USAGE: u8 = 2;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [destination] = arguments.as_slice() else {
        return fail(USAGE, "usage: assemble-package <destination>");
    };

    let root = repository_root();
    let destination = Path::new(destination);
    match assemble_package_tree(&root.join(PACKAGE_SOURCE_PATH), &root, destination) {
        Ok(package) => {
            println!("assembled: {}", destination.display());
            println!(
                "OK: {} carries {} files drawn from {PACKAGE_SOURCE_PATH} and the \
                 repository files its live assets name",
                package.model().id,
                package.file_count()
            );
            ExitCode::SUCCESS
        }
        Err(error) => fail(1, &error.to_string()),
    }
}

fn fail(code: u8, message: &str) -> ExitCode {
    eprintln!("assemble-package: {message}");
    ExitCode::from(code)
}

/// The checkout these sources were compiled from, which is the repository whose
/// package sources and live files the assembly draws from.
fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
