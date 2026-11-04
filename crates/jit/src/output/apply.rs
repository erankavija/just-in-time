//! Human rendering of the `jit apply` response.

use super::{HumanRender, TemplateApplyResponse};

impl HumanRender for TemplateApplyResponse {
    fn render_human(&self) -> String {
        if self.quiet {
            return self
                .created_node_ids_by_role
                .values()
                .map(|id| format!("{id}\n"))
                .collect();
        }
        let roles = self
            .created_node_ids_by_role
            .iter()
            .map(|(role, id)| format!("{role}={id}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("{}: {roles}\n", self.message())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(quiet: bool) -> TemplateApplyResponse {
        TemplateApplyResponse {
            template: "plan".to_owned(),
            container: crate::test_utils::issue_id(2),
            anchor_bindings: Default::default(),
            created_node_ids_by_role: [("review".to_owned(), crate::test_utils::issue_id(1))]
                .into(),
            anchor_dependency_snapshots: Default::default(),
            created_issues: Default::default(),
            quiet,
        }
    }

    #[test]
    fn test_render_human_quiet_lists_only_the_created_ids() {
        let response = response(true);

        let expected: String = response
            .created_node_ids_by_role
            .values()
            .map(|id| format!("{id}\n"))
            .collect();
        assert_eq!(response.render_human(), expected);
    }

    #[test]
    fn test_render_human_names_template_container_and_each_role() {
        let response = response(false);

        let rendered = response.render_human();
        assert!(rendered.starts_with(&response.message()));
        assert!(rendered.contains(&format!(
            "review={}",
            response.created_node_ids_by_role["review"]
        )));
    }
}
