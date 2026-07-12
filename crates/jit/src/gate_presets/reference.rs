//! Projection of the built-in gate presets into a committed markdown reference.
//!
//! The presets themselves are the source of truth: [`BuiltinPresets::load`]
//! returns every preset the binary ships, and [`render_reference_markdown`]
//! projects those definitions — preset names and descriptions, and for each
//! bundled gate its key, title, stage, mode, description, and checker
//! configuration — into the committed reference [`REFERENCE_PATH`]. A
//! conformance test in this module asserts the committed copy equals the
//! projection, so changing a preset or one of its gate definitions without
//! regenerating the reference fails the test suite
//! (`@/inv/single-source-prose`).
//!
//! The rendering is deterministic: presets are sorted by name and checker
//! environment variables by key, so no `HashMap` iteration order reaches the
//! output.

use super::{BuiltinPresets, GatePresetDefinition, GateTemplate};
use crate::domain::GateChecker;
use anyhow::Result;

/// Repo-relative path of the committed reference that projects the built-in
/// presets.
///
/// # Examples
///
/// ```
/// use jit::gate_presets::REFERENCE_PATH;
///
/// assert!(REFERENCE_PATH.ends_with("gate-presets.md"));
/// ```
pub const REFERENCE_PATH: &str = "docs/reference/gate-presets.md";

/// Every built-in preset, sorted by name.
///
/// The sort makes the projection deterministic: [`BuiltinPresets::load`] returns
/// a `HashMap`, whose iteration order is unspecified.
fn presets_sorted() -> Result<Vec<GatePresetDefinition>> {
    let mut presets: Vec<GatePresetDefinition> = BuiltinPresets::load()?.into_values().collect();
    presets.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(presets)
}

/// Escape the markdown table cell separator so a `|` inside a command,
/// description, or environment value cannot break the rendered row.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// Render one gate's checker configuration as a table cell: every field of the
/// checker, or the manual-gate note when the gate carries none.
///
/// [`GateChecker`] has a single variant, `exec`, and every one of its fields —
/// command, timeout, working directory, environment, context passing, inline
/// prompt, prompt file — is projected, so the cell fully determines how the gate
/// runs.
fn checker_cell(gate: &GateTemplate) -> String {
    let Some(GateChecker::Exec {
        command,
        timeout_seconds,
        working_dir,
        env,
        pass_context,
        prompt,
        prompt_file,
    }) = gate.checker.as_ref()
    else {
        // `GatePresetDefinition::validate` rejects an auto gate without a
        // checker, so a checker-less gate is always manual: it is passed by
        // attestation (`jit gate evaluate <id> <gate>`), never by a command.
        return "none — manual attestation".to_string();
    };

    // Sort the environment by key: `env` is a `HashMap`.
    let mut env_pairs: Vec<(&String, &String)> = env.iter().collect();
    env_pairs.sort_by(|a, b| a.0.cmp(b.0));
    let env_rendered = if env_pairs.is_empty() {
        "empty".to_string()
    } else {
        env_pairs
            .iter()
            .map(|(key, value)| format!("`{}={}`", cell(key), cell(value)))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let optional = |value: Option<&String>| match value {
        Some(value) => format!("`{}`", cell(value)),
        None => "unset".to_string(),
    };

    format!(
        "`exec` — command `{command}`; timeout {timeout_seconds}s; working dir: {working_dir}; \
         env: {env_rendered}; context passed: {pass_context}; prompt: {prompt}; prompt file: \
         {prompt_file}",
        command = cell(command),
        working_dir = optional(working_dir.as_ref()),
        pass_context = if *pass_context { "yes" } else { "no" },
        prompt = optional(prompt.as_ref()),
        prompt_file = optional(prompt_file.as_ref()),
    )
}

/// Render one preset as a section: its name, description, and the table of the
/// gates it bundles.
fn preset_section(preset: &GatePresetDefinition) -> String {
    // Gates keep their authored order — the order `jit gate preset apply` walks
    // them in, and the order `jit gate preset show` prints.
    let rows = preset
        .gates
        .iter()
        .map(|gate| {
            format!(
                "| `{key}` | {title} | {stage} | {mode} | {description} | {checker} |",
                key = cell(&gate.key),
                title = cell(&gate.title),
                stage = gate.stage.as_str(),
                mode = gate.mode.as_str(),
                description = cell(&gate.description),
                checker = checker_cell(gate),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "## `{name}`\n\
         \n\
         {description}\n\
         \n\
         | Gate key | Title | Stage | Mode | Description | Checker |\n\
         | --- | --- | --- | --- | --- | --- |\n\
         {rows}\n",
        name = cell(&preset.name),
        description = cell(&preset.description),
    )
}

/// Render the built-in gate presets as the committed markdown reference.
///
/// The returned string is the full contents of [`REFERENCE_PATH`]. Every preset
/// name, description, gate key, title, stage, mode, gate description, and
/// checker field is read from [`BuiltinPresets::load`], so the page cannot drift
/// from the definitions the binary applies. The conformance test in this module
/// asserts the committed file equals this output.
///
/// # Errors
///
/// Propagates a [`BuiltinPresets::load`] failure (a built-in preset that fails
/// its own `validate`).
///
/// # Examples
///
/// ```
/// use jit::gate_presets::{render_reference_markdown, BuiltinPresets};
///
/// let doc = render_reference_markdown().unwrap();
/// assert!(doc.starts_with("<!--"));
/// assert!(doc.contains("# Built-in Gate Presets"));
///
/// // Every shipped preset has a section, in sorted order.
/// for name in BuiltinPresets::names() {
///     assert!(doc.contains(&format!("## `{name}`")), "missing section for {name}");
/// }
/// let breakdown = doc.find("## `breakdown-review`").unwrap();
/// let coverage = doc.find("## `coverage-preview`").unwrap();
/// assert!(breakdown < coverage);
/// ```
pub fn render_reference_markdown() -> Result<String> {
    let presets = presets_sorted()?;

    let summary = presets
        .iter()
        .map(|preset| {
            format!(
                "| [`{name}`](#{anchor}) | {description} | {count} |",
                name = cell(&preset.name),
                anchor = preset.name,
                description = cell(&preset.description),
                count = preset.gates.len(),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let sections = presets
        .iter()
        .map(preset_section)
        .collect::<Vec<_>>()
        .join("\n");

    Ok(format!(
        "<!-- Generated from `crate::gate_presets::reference` — do not edit by hand. -->\n\
         \n\
         # Built-in Gate Presets\n\
         \n\
         > **Diátaxis Type:** Reference\n\
         \n\
         The gate presets the `jit` binary ships. A preset is a named bundle of gate\n\
         definitions; `jit gate preset apply <preset> <id>...` inserts each bundled gate\n\
         into the project's gate registry (`.jit/gates.toml`) under its key — for keys the\n\
         registry does not already carry, and, with `--timeout <seconds>`, overwriting the\n\
         key with the overridden checker timeout — and then adds those keys to each issue's\n\
         required gates. `--no-precheck`, `--no-postcheck`, and `--except <key>` narrow which\n\
         of the preset's gates are applied. Each gate materializes into the registry with\n\
         `version = 1`, `priority = 100`, and `auto` set from its mode.\n\
         \n\
         This page is generated from the preset definitions in\n\
         `crates/jit/src/gate_presets/` and lists what the binary carries — not what any\n\
         repository has configured. `jit init` writes an empty gate registry, so nothing\n\
         below reaches a project until `jit gate preset apply` runs. The gates a project\n\
         actually enforces live in its own `.jit/gates.toml`, its settings in\n\
         `.jit/config.toml`; render those with `jit reference render` (see\n\
         [Rules and Gates](rules-and-gates.md)).\n\
         \n\
         A project can also define its own presets: `jit gate preset create <issue> <name>`\n\
         captures an issue's gates into `.jit/config/gate-presets/<name>.json`, and every\n\
         JSON file in that directory loads alongside the built-ins. `jit gate preset create`\n\
         rejects a built-in name; a hand-authored file that reuses one shadows the built-in\n\
         for `jit gate preset show` and `apply`, and `jit gate preset list` then reports that\n\
         name as project-local instead of `[builtin]`.\n\
         \n\
         | Preset | Description | Gates |\n\
         | --- | --- | --- |\n\
         {summary}\n\
         \n\
         {sections}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::GateMode;
    use std::collections::HashSet;
    use std::path::PathBuf;

    /// Absolute path of the committed reference, resolved from the crate root so
    /// the test is independent of the process working directory.
    fn reference_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(REFERENCE_PATH)
    }

    /// REQ-03 conformance: the committed reference must equal the projection of
    /// the current built-in presets. Changing a preset or any of its gate
    /// definitions (key, title, stage, mode, description, checker) without
    /// regenerating the reference fails here.
    #[test]
    fn test_committed_reference_matches_projection() {
        let committed = std::fs::read_to_string(reference_path())
            .expect("committed gate-presets reference should exist");
        assert_eq!(
            committed,
            render_reference_markdown().unwrap(),
            "{REFERENCE_PATH} is stale — regenerate it from the built-in presets \
             (run: cargo test -p jit gate_presets::reference -- --ignored regenerate)"
        );
    }

    /// Regenerate the committed reference from the built-in presets. Ignored by
    /// default; run explicitly after changing a preset:
    ///   cargo test -p jit gate_presets::reference -- --ignored regenerate
    ///
    /// Writes via the temp-file + atomic-rename pattern (`@/inv/atomic-writes`).
    #[test]
    #[ignore = "writes the committed reference; run explicitly to regenerate"]
    fn test_regenerate_reference_writes_committed_doc() {
        let path = reference_path();
        let tmp = path.with_extension("md.tmp");
        std::fs::write(&tmp, render_reference_markdown().unwrap())
            .expect("should write the gate-presets temp file");
        std::fs::rename(&tmp, &path).expect("should atomically replace the gate-presets reference");
    }

    /// The projection covers every shipped preset, and every gate of each one,
    /// with its stage, mode, and checker command.
    #[test]
    fn test_render_covers_every_preset_and_gate() {
        let doc = render_reference_markdown().unwrap();
        for preset in presets_sorted().unwrap() {
            assert!(
                doc.contains(&format!("## `{}`", preset.name)),
                "no section for preset {}",
                preset.name
            );
            assert!(doc.contains(&preset.description), "{}", preset.name);
            for gate in &preset.gates {
                assert!(
                    doc.contains(&format!("| `{}` |", gate.key)),
                    "preset {} gate {} missing",
                    preset.name,
                    gate.key
                );
                assert!(doc.contains(gate.stage.as_str()));
                assert!(doc.contains(gate.mode.as_str()));
                match gate.checker.as_ref() {
                    Some(GateChecker::Exec {
                        command,
                        timeout_seconds,
                        ..
                    }) => {
                        assert!(
                            doc.contains(&format!(
                                "command `{command}`; timeout {timeout_seconds}s"
                            )),
                            "checker of {} missing",
                            gate.key
                        );
                    }
                    None => assert_eq!(gate.mode, GateMode::Manual),
                }
            }
        }
    }

    /// The rendered set of presets is exactly `BuiltinPresets::names()`: the
    /// projection can neither miss a shipped preset nor invent one.
    #[test]
    fn test_render_matches_builtin_names() {
        let rendered: HashSet<String> = presets_sorted()
            .unwrap()
            .into_iter()
            .map(|preset| preset.name)
            .collect();
        let declared: HashSet<String> = BuiltinPresets::names().into_iter().collect();
        assert_eq!(rendered, declared);
    }

    /// Rendering is deterministic: repeated renders are byte-identical despite
    /// the `HashMap`s behind the preset registry and each checker's environment.
    #[test]
    fn test_render_is_deterministic() {
        let first = render_reference_markdown().unwrap();
        for _ in 0..8 {
            assert_eq!(first, render_reference_markdown().unwrap());
        }
    }

    /// Checker environments are projected, sorted by key.
    #[test]
    fn test_checker_cell_renders_every_field() {
        let gate = GateTemplate {
            key: "k".to_string(),
            title: "t".to_string(),
            description: "d".to_string(),
            stage: crate::domain::GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: Some(GateChecker::Exec {
                command: "run.sh".to_string(),
                timeout_seconds: 42,
                working_dir: Some("sub".to_string()),
                env: [
                    ("Z".to_string(), "last".to_string()),
                    ("A".to_string(), "first".to_string()),
                ]
                .into_iter()
                .collect(),
                pass_context: true,
                prompt: Some("inline".to_string()),
                prompt_file: Some("p.md".to_string()),
            }),
        };
        assert_eq!(
            checker_cell(&gate),
            "`exec` — command `run.sh`; timeout 42s; working dir: `sub`; env: `A=first`, \
             `Z=last`; context passed: yes; prompt: `inline`; prompt file: `p.md`"
        );

        // An absent optional field renders as `unset`, an empty environment as
        // `empty` — every checker field reaches the cell either way.
        let bare = GateTemplate {
            checker: Some(GateChecker::Exec {
                command: "run.sh".to_string(),
                timeout_seconds: 1,
                working_dir: None,
                env: std::collections::HashMap::new(),
                pass_context: false,
                prompt: None,
                prompt_file: None,
            }),
            ..gate
        };
        assert_eq!(
            checker_cell(&bare),
            "`exec` — command `run.sh`; timeout 1s; working dir: unset; env: empty; \
             context passed: no; prompt: unset; prompt file: unset"
        );
    }

    /// A manual gate carries no checker and says so.
    #[test]
    fn test_checker_cell_manual_gate() {
        let gate = GateTemplate {
            key: "k".to_string(),
            title: "t".to_string(),
            description: "d".to_string(),
            stage: crate::domain::GateStage::Precheck,
            mode: GateMode::Manual,
            checker: None,
        };
        assert_eq!(checker_cell(&gate), "none — manual attestation");
    }

    /// A `|` in a projected value is escaped so it cannot break the table row.
    #[test]
    fn test_cell_escapes_table_separator() {
        assert_eq!(cell("a | b"), "a \\| b");
    }
}
