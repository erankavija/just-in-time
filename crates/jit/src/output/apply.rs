//! Human rendering of the `jit apply` response.

use super::{HumanRender, OutputMode, TemplateApplyResponse};
use crate::commands::UpstreamMoveReport;

impl HumanRender for TemplateApplyResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        if mode.quiet {
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
        let moved = self
            .upstream_move
            .as_ref()
            .map(render_upstream_move)
            .unwrap_or_default();
        format!("{}: {roles}\n{moved}", self.message())
    }
}

fn render_upstream_move(report: &UpstreamMoveReport) -> String {
    let lines: String = report
        .moves
        .iter()
        .map(|moved| {
            format!(
                "  {} now blocks {} {}\n",
                moved.dependency, moved.role, moved.node
            )
        })
        .collect();
    format!(
        "Moved {} existing dependencies of container {} onto created nodes; audit them:\n{lines}",
        report.moves.len(),
        report.container
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::UpstreamMove;
    use crate::test_utils::issue_id;

    fn mode(quiet: bool) -> OutputMode {
        OutputMode { json: false, quiet }
    }

    fn response() -> TemplateApplyResponse {
        TemplateApplyResponse {
            template: "plan".to_owned(),
            container: issue_id(2),
            anchor_bindings: Default::default(),
            created_node_ids_by_role: [("review".to_owned(), issue_id(1))].into(),
            anchor_dependency_snapshots: Default::default(),
            created_issues: Default::default(),
            upstream_move: None,
        }
    }

    fn moved_response() -> TemplateApplyResponse {
        let mut response = response();
        response.upstream_move = Some(UpstreamMoveReport {
            container: response.container.clone(),
            moves: [issue_id(3), issue_id(4)]
                .map(|dependency| UpstreamMove {
                    dependency,
                    role: "review".to_owned(),
                    node: issue_id(1),
                })
                .to_vec(),
        });
        response
    }

    #[test]
    fn test_render_human_quiet_lists_only_the_created_ids() {
        let response = moved_response();

        let expected: String = response
            .created_node_ids_by_role
            .values()
            .map(|id| format!("{id}\n"))
            .collect();
        assert_eq!(response.render_human(mode(true)), expected);
    }

    #[test]
    fn test_render_human_names_template_container_and_each_role() {
        let response = response();

        let rendered = response.render_human(mode(false));
        assert!(rendered.starts_with(&response.message()));
        assert!(rendered.contains(&format!(
            "review={}",
            response.created_node_ids_by_role["review"]
        )));
    }

    #[test]
    fn test_render_human_without_upstream_move_prints_only_the_applied_line() {
        let response = response();

        assert_eq!(response.render_human(mode(false)).lines().count(), 1);
    }

    #[test]
    fn test_render_human_upstream_move_names_container_and_each_moved_edge() {
        let response = moved_response();
        let report = response.upstream_move.as_ref().unwrap();

        let rendered = response.render_human(mode(false));
        assert!(rendered.contains(&report.container.to_string()));
        assert!(report.moves.iter().all(|moved| rendered.contains(&format!(
            "{} now blocks {} {}",
            moved.dependency, moved.role, moved.node
        ))));
    }
}
