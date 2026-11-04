//! Human rendering for `jit hooks` responses.

use super::{HumanRender, OutputMode};
use schemars::JsonSchema;
use serde::Serialize;

/// The successful result of installing repository hooks.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HooksInstallResponse {
    pub hooks_dir: String,
    pub installed: Vec<String>,
    pub skipped: Vec<String>,
}

fn section(title: &str, bullet: &str, items: &[String]) -> String {
    let content = items.join(&format!("\n{bullet}"));
    items
        .first()
        .map(|_| format!("\n{title}:\n{bullet}{content}\n"))
        .unwrap_or_default()
}

impl HumanRender for HooksInstallResponse {
    fn render_human(&self, _: OutputMode) -> String {
        format!(
            "Installed hooks to: {}\n{}{}\nHooks are now active. Leases are enforced when a mutation is written, not at commit.\n",
            self.hooks_dir,
            section("Installed", "  ", &self.installed),
            section("Skipped (already exist)", "  - ", &self.skipped)
        )
    }
}
