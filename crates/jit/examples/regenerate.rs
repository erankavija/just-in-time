//! Regenerate one artifact this repository generates into its own checkout.
//!
//! Invoke it through the artifact's own entry point in `scripts/`, which is the
//! command the artifact's drift assertion names. Each render lives in
//! `jit::generated_artifacts`, which the drift assertions read too, so the
//! repair and the assertion cannot disagree about what an artifact should hold;
//! this file supplies only the I/O around them.
//!
//! The declarations come from the crate cargo compiles out of the checkout this
//! example runs against, so there is no installed-binary currency question to
//! settle first — unlike the shipped policy regions, whose values can only be
//! read out of an already-installed binary, and whose entry point therefore owns
//! its own render.
//!
//! Usage:
//!   regenerate <artifact>
//!
//! Exit codes:
//!   0 — the artifact holds its rendered values
//!   1 — the render or the publication failed
//!   2 — a usage error

use jit::generated_artifacts::{generated_artifact, ArtifactRender, GENERATED_ARTIFACTS};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Exit code for a usage error, matching the entry-point scripts' convention.
const USAGE: u8 = 2;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [name] = arguments.as_slice() else {
        return fail(USAGE, &format!("usage: regenerate <artifact>\n{}", known()));
    };

    let Some(artifact) = generated_artifact(name) else {
        return fail(USAGE, &format!("no artifact named '{name}'\n{}", known()));
    };

    let ArtifactRender::InCrate { target, render } = &artifact.render else {
        return fail(
            USAGE,
            &format!(
                "'{name}' is rendered by its own entry point: run {}",
                artifact.generator
            ),
        );
    };

    match regenerate(&repository_root(), target, *render) {
        Ok(true) => report(&format!(
            "updated: {target}\nOK: {target} rendered from {}",
            artifact.holds
        )),
        Ok(false) => report(&format!("OK: {target} already carries {}", artifact.holds)),
        Err(message) => fail(1, &message),
    }
}

/// The artifact names this entry point accepts, one per line.
fn known() -> String {
    GENERATED_ARTIFACTS
        .iter()
        .map(|artifact| format!("  {} ({})", artifact.name, artifact.generator))
        .collect::<Vec<_>>()
        .join("\n")
}

fn report(message: &str) -> ExitCode {
    println!("{message}");
    ExitCode::SUCCESS
}

fn fail(code: u8, message: &str) -> ExitCode {
    eprintln!("regenerate: {message}");
    ExitCode::from(code)
}

/// The checkout these sources were compiled from, which is the repository whose
/// artifacts the compiled declarations belong to.
fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Publish `target`'s rendered bytes, reporting whether they differed from the
/// committed ones.
fn regenerate(
    root: &Path,
    target: &str,
    render: fn(&[u8]) -> Result<Vec<u8>, String>,
) -> Result<bool, String> {
    let path = root.join(target);
    let committed =
        std::fs::read(&path).map_err(|error| format!("cannot read {target}: {error}"))?;
    let rendered = render(&committed)?;
    if rendered == committed {
        return Ok(false);
    }

    // Publish through a staged file and one rename, so a reader loading the
    // artifact while this runs sees one whole version or the other. The staged
    // file takes the target's own permissions before the rename: a temporary
    // file is created private to its owner, and inheriting that would leave the
    // artifact unreadable to everyone else.
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
    Ok(true)
}
