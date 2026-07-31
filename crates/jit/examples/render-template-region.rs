//! Render this repository's graph-template declaration from the packaged
//! `jit-dogfood` template contributions, which are its authority
//! (`@/issue/e204e63d/decision/D-1`).
//!
//! Invoke it through `./scripts/generate-template-region.sh`, which is the
//! entry point the registry file and the drift assertion both name. The whole
//! render lives in `jit::profile::template_region`, so the generator and the
//! assertion that guards it cannot disagree about what the region should hold;
//! this file supplies only the I/O around it.
//!
//! The declarations come from the package embedded in *this* build, which cargo
//! compiles from the checkout the example runs against, so there is no
//! installed-binary currency question to settle first — unlike the shipped
//! policy regions, whose values can only be read out of an already-installed
//! binary.
//!
//! Exit codes: 0 — the region holds the packaged declarations; 1 — the render
//! or the publication failed.

use jit::profile::template_region::{outside_template_region, render_template_registry};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    match render(&repository_root()) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("render-template-region: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The checkout these sources were compiled from, which is the repository whose
/// registry the packaged declarations belong to.
fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Publish the rendered registry, reporting what the run did.
fn render(root: &Path) -> Result<String, String> {
    let target = ".jit/templates.toml";
    let path = root.join(target);
    let existing =
        std::fs::read(&path).map_err(|error| format!("cannot read {target}: {error}"))?;
    let rendered = render_template_registry(&existing).map_err(|error| error.to_string())?;

    // A render that moved an authored byte is a defect in the splice, not an
    // edit to publish: the delimiters bound what the package owns.
    if outside_template_region(&rendered).map_err(|error| error.to_string())?
        != outside_template_region(&existing).map_err(|error| error.to_string())?
    {
        return Err(format!(
            "the render changed {target} outside the delimiters"
        ));
    }

    if rendered == existing {
        return Ok(format!(
            "OK: {target} already carries the packaged template declarations"
        ));
    }

    // Publish through a staged file and one rename, so a reader loading the
    // registry while this runs sees one whole version or the other. The staged
    // file takes the target's own permissions before the rename: a temporary
    // file is created private to its owner, and inheriting that would leave the
    // registry unreadable to everyone else.
    let mut staged = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| format!("{target} has no parent directory"))?,
    )
    .map_err(|error| format!("cannot stage {target}: {error}"))?;
    staged
        .write_all(&rendered)
        .map_err(|error| format!("cannot write the staged {target}: {error}"))?;
    let permissions = std::fs::metadata(&path)
        .map_err(|error| format!("cannot read the permissions of {target}: {error}"))?
        .permissions();
    staged
        .as_file()
        .set_permissions(permissions)
        .map_err(|error| format!("cannot set the permissions of the staged {target}: {error}"))?;
    staged
        .persist(&path)
        .map_err(|error| format!("cannot publish {target}: {error}"))?;
    Ok(format!(
        "updated: {target}\nOK: {target} rendered from the packaged template declarations"
    ))
}
