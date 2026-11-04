//! Success payloads for dependency edits.

use super::{HumanRender, OutputMode};
use crate::domain::identity::IssueId;
use schemars::JsonSchema;
use serde::Serialize;
use std::fmt::Write;

#[derive(Debug, Serialize, JsonSchema)]
pub struct DependencyAddResponse {
    pub from_id: IssueId,
    pub added: Vec<IssueId>,
    pub already_exist: Vec<IssueId>,
    pub skipped: Vec<(IssueId, String)>,
}

impl HumanRender for DependencyAddResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let mut text = String::new();
        if !self.added.is_empty() && !mode.quiet {
            let _ = writeln!(
                text,
                "Added {} dependenc{}:",
                self.added.len(),
                if self.added.len() == 1 { "y" } else { "ies" }
            );
        }
        self.added.iter().for_each(|id| {
            let _ = writeln!(text, "  • {} → {}", self.from_id, id);
        });
        if !self.already_exist.is_empty() {
            let _ = writeln!(text, "Already exist ({}):", self.already_exist.len());
            self.already_exist.iter().for_each(|id| {
                let _ = writeln!(text, "  • {}", id);
            });
        }
        if !self.skipped.is_empty() {
            let _ = writeln!(text, "Skipped ({}):", self.skipped.len());
            self.skipped.iter().for_each(|(id, reason)| {
                let _ = writeln!(text, "  • {}: {}", id, reason);
            });
        }
        text
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DependencyRemoveResponse {
    pub from_id: IssueId,
    pub removed: Vec<IssueId>,
    pub not_found: Vec<String>,
}

impl HumanRender for DependencyRemoveResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let mut text = String::new();
        if !self.removed.is_empty() && !mode.quiet {
            let _ = writeln!(
                text,
                "Removed {} dependenc{}:",
                self.removed.len(),
                if self.removed.len() == 1 { "y" } else { "ies" }
            );
        }
        self.removed.iter().for_each(|id| {
            let _ = writeln!(text, "  • {}", id);
        });
        if !self.not_found.is_empty() {
            let _ = writeln!(text, "Not found ({}):", self.not_found.len());
            self.not_found.iter().for_each(|id| {
                let _ = writeln!(text, "  • {}", id);
            });
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_add_render_human_preserves_sections_and_quiet_detail_rows() {
        let from_id = IssueId::from_uuid(uuid::Uuid::new_v4());
        let target = IssueId::from_uuid(uuid::Uuid::new_v4());
        let response = DependencyAddResponse {
            from_id: from_id.clone(),
            added: vec![target.clone()],
            already_exist: vec![target.clone()],
            skipped: vec![(target.clone(), "redundant".to_owned())],
        };
        let details = format!(
            "  • {from_id} → {target}\nAlready exist (1):\n  • {target}\nSkipped (1):\n  • {target}: redundant\n"
        );
        assert_eq!(
            response.render_human(OutputMode::default()),
            format!("Added 1 dependency:\n{details}")
        );
        assert_eq!(
            response.render_human(OutputMode {
                quiet: true,
                json: false
            }),
            details
        );
        let multiple = DependencyAddResponse {
            added: vec![target.clone(), target],
            already_exist: Vec::new(),
            skipped: Vec::new(),
            ..response
        };
        assert!(multiple
            .render_human(OutputMode::default())
            .starts_with("Added 2 dependencies:\n"));
        let empty = DependencyAddResponse {
            added: Vec::new(),
            ..multiple
        };
        assert!(empty.render_human(OutputMode::default()).is_empty());
    }

    #[test]
    fn test_dependency_remove_render_human_preserves_sections_and_quiet_detail_rows() {
        let from_id = IssueId::from_uuid(uuid::Uuid::new_v4());
        let target = IssueId::from_uuid(uuid::Uuid::new_v4());
        let response = DependencyRemoveResponse {
            from_id,
            removed: vec![target.clone()],
            not_found: vec!["missing".to_owned()],
        };
        let details = format!("  • {target}\nNot found (1):\n  • missing\n");
        assert_eq!(
            response.render_human(OutputMode::default()),
            format!("Removed 1 dependency:\n{details}")
        );
        assert_eq!(
            response.render_human(OutputMode {
                quiet: true,
                json: false
            }),
            details
        );
        let multiple = DependencyRemoveResponse {
            removed: vec![target.clone(), target],
            not_found: Vec::new(),
            ..response
        };
        assert!(multiple
            .render_human(OutputMode::default())
            .starts_with("Removed 2 dependencies:\n"));
        let empty = DependencyRemoveResponse {
            removed: Vec::new(),
            ..multiple
        };
        assert!(empty.render_human(OutputMode::default()).is_empty());
    }
}
