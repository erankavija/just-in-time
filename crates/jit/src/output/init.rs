//! Human rendering of the `jit init` response.

use super::{HumanRender, InitResponse, OutputMode};
use crate::repository_state::VirtualPath;

impl InitResponse {
    /// The `--json` envelope's `message`.
    pub fn message(&self) -> String {
        match &self.repository_id {
            Some(id) => format!("Initialized jit repository (worktree: {id})"),
            None => "Initialized jit repository".to_owned(),
        }
    }
}

impl HumanRender for InitResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        if mode.quiet {
            return String::new();
        }
        let rules_path = VirtualPath::RULES.repository_relative();
        let scaffolded = if self.created_paths.contains(&rules_path) {
            "Scaffolded .jit/rules.toml\n"
        } else {
            ""
        };
        format!("{scaffolded}{}\n", self.message())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::GitattributesStatus;

    fn mode(quiet: bool) -> OutputMode {
        OutputMode { json: false, quiet }
    }

    fn response(repository_id: Option<&str>, created: &[String]) -> InitResponse {
        InitResponse {
            repository_root: "/repo".to_owned(),
            data_dir: "/repo/.jit".to_owned(),
            repository_id: repository_id.map(str::to_owned),
            gitattributes_status: GitattributesStatus::Unchanged,
            created_paths: created.to_vec(),
            modified_paths: Vec::new(),
            profile: None,
        }
    }

    #[test]
    fn test_render_human_quiet_prints_nothing() {
        assert_eq!(response(None, &[]).render_human(mode(true)), "");
    }

    #[test]
    fn test_render_human_names_the_worktree_identity_in_the_message() {
        let response = response(Some("wt:1234"), &[]);

        assert!(response.render_human(mode(false)).contains("wt:1234"));
        assert!(response.message().contains("wt:1234"));
    }

    #[test]
    fn test_render_human_reports_scaffolded_rules_only_when_this_run_created_them() {
        let created = [VirtualPath::RULES.repository_relative()];

        assert!(response(None, &created)
            .render_human(mode(false))
            .starts_with("Scaffolded .jit/rules.toml\n"));
        assert!(!response(None, &[])
            .render_human(mode(false))
            .contains("Scaffolded"));
    }
}
