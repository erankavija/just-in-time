//! Unified, read-only archive planning commands.

use super::CommandExecutor;
use crate::domain::artifact_classifier::{
    classify_artifacts, ArtifactClassificationInventory, ArtifactClassificationPolicy,
};
use crate::domain::artifact_inventory::{inventory_explicit_roots, ExplicitRootTarget};
use crate::domain::artifact_plan::{ArtifactPlan, BlockerCode, PlanBlocker};
use crate::storage::{
    collect_artifact_classification_facts, discover_artifact_dependencies,
    discover_repository_embedded_owners, GitRevisionResolver, IssueStore,
};
use anyhow::{Context, Result};
use std::collections::BTreeSet;

enum ArchiveTarget<'a> {
    Document(&'a str),
    Container(&'a str),
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Build a complete non-mutating plan for one arbitrary repository document.
    pub fn preview_archive_document(&self, path: &str) -> Result<ArtifactPlan> {
        self.plan_archive_target(ArchiveTarget::Document(path))
    }

    /// Build a complete non-mutating plan for one resolved container subtree.
    pub fn preview_archive_container(&self, id: &str) -> Result<ArtifactPlan> {
        self.plan_archive_target(ArchiveTarget::Container(id))
    }

    fn plan_archive_target(&self, target: ArchiveTarget<'_>) -> Result<ArtifactPlan> {
        let config = self.config_manager.load()?;
        let hierarchy = crate::config_manager::get_hierarchy_config(&self.storage)?;
        let issues = self.storage.list_issues()?;
        let repo_root = self
            .storage
            .root()
            .parent()
            .context("archive planning requires .jit beneath a repository root")?;
        let resolver = GitRevisionResolver::new(repo_root);

        let root_container_id = match target {
            ArchiveTarget::Document(_) => None,
            ArchiveTarget::Container(id) => Some(self.storage.resolve_issue_id(id)?),
        };
        let explicit_target = match (&target, root_container_id.as_deref()) {
            (ArchiveTarget::Document(path), _) => ExplicitRootTarget::Document(path),
            (ArchiveTarget::Container(_), Some(id)) => ExplicitRootTarget::Container(id),
            (ArchiveTarget::Container(_), None) => unreachable!("container id was resolved"),
        };
        let inventory = inventory_explicit_roots(&issues, &hierarchy, explicit_target, &resolver)?;
        let discovered = discover_artifact_dependencies(&self.storage, inventory)?;
        let member_ids = discovered
            .member_ids()
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let embedded_owners =
            discover_repository_embedded_owners(&self.storage, &issues, &member_ids)?;
        let (plan_target, artifacts, mut blockers) = discovered.into_plan_parts();

        if let Some(container_id) = root_container_id.as_deref() {
            if issues
                .iter()
                .find(|issue| issue.id == container_id)
                .is_some_and(|issue| !issue.state.is_terminal())
            {
                blockers.push(PlanBlocker::new(
                    BlockerCode::NonTerminalTarget,
                    None::<String>,
                ));
            }
        }

        let policy =
            ArtifactClassificationPolicy::from_documentation(config.documentation.as_ref());
        let facts = collect_artifact_classification_facts(
            &self.storage,
            &plan_target,
            &artifacts,
            &policy,
            embedded_owners,
        )?;
        classify_artifacts(
            ArtifactClassificationInventory::new(plan_target, artifacts, blockers),
            policy,
            facts,
        )
        .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DocumentReference, Issue, State};
    use crate::storage::JsonFileStorage;
    use std::fs;
    use tempfile::TempDir;

    fn configured_repo() -> (TempDir, CommandExecutor<JsonFileStorage>, String) {
        let repo = TempDir::new().unwrap();
        let jit = repo.path().join(".jit");
        let storage = JsonFileStorage::new(&jit);
        storage.init().unwrap();
        fs::write(
            jit.join("config.toml"),
            r#"
[documentation]
managed_paths = ["fixtures"]
permanent_paths = ["docs"]
archive_root = "archive"

[type_hierarchy]
types = { epic = 1, task = 2 }
[type_hierarchy.label_associations]
epic = "epic"
"#,
        )
        .unwrap();

        fs::create_dir_all(repo.path().join("fixtures/bundle/theme")).unwrap();
        fs::write(
            repo.path().join("fixtures/readme.md"),
            "[page](bundle/index.html) [external](https://example.com)",
        )
        .unwrap();
        fs::write(
            repo.path().join("fixtures/bundle/index.html"),
            r#"<link href="theme/base.css"><img src="image.svg">"#,
        )
        .unwrap();
        fs::write(
            repo.path().join("fixtures/bundle/theme/base.css"),
            r#"@import "nested.css"; body { background: url("../../image.png"); }"#,
        )
        .unwrap();
        fs::write(repo.path().join("fixtures/bundle/theme/nested.css"), "a{}").unwrap();
        fs::write(repo.path().join("fixtures/bundle/image.svg"), "<svg/>").unwrap();
        fs::write(repo.path().join("fixtures/bundle/image.png"), b"png").unwrap();
        fs::write(repo.path().join("fixtures/data.csv"), "a,b\n1,2\n").unwrap();

        let mut epic = Issue::new("Archive fixture".into(), String::new());
        epic.state = State::Done;
        epic.labels = vec!["type:epic".into()];
        epic.documents = [
            "fixtures/readme.md",
            "fixtures/bundle/index.html",
            "fixtures/bundle/theme/base.css",
            "fixtures/data.csv",
            "fixtures/bundle/image.png",
            "fixtures/bundle/image.svg",
        ]
        .into_iter()
        .map(|path| DocumentReference::new(path.into()))
        .collect();
        let id = epic.id.clone();
        storage.save_issue(epic).unwrap();
        (repo, CommandExecutor::new(storage), id)
    }

    #[test]
    fn test_preview_container_is_deterministic_complete_and_non_mutating() {
        let (repo, executor, id) = configured_repo();
        let events_before = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();
        let issues_before = executor.storage.list_issues().unwrap();

        let first = executor.preview_archive_container(&id).unwrap();
        let second = executor.preview_archive_container(&id).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.schema_version(), 1);
        assert!(first.eligible());
        for path in [
            "fixtures/readme.md",
            "fixtures/bundle/index.html",
            "fixtures/bundle/theme/base.css",
            "fixtures/data.csv",
            "fixtures/bundle/image.png",
            "fixtures/bundle/image.svg",
        ] {
            assert!(first
                .artifacts()
                .iter()
                .any(|artifact| artifact.source() == path));
        }
        for path in [
            "fixtures/data.csv",
            "fixtures/bundle/image.png",
            "fixtures/bundle/image.svg",
        ] {
            assert_eq!(
                first
                    .artifacts()
                    .iter()
                    .find(|artifact| artifact.source() == path)
                    .unwrap()
                    .format(),
                None,
                "opaque root {path} must remain inventoried with null format"
            );
        }
        assert!(!repo.path().join("archive").exists());
        assert_eq!(executor.storage.list_issues().unwrap(), issues_before);
        assert_eq!(
            fs::read(repo.path().join(".jit/events.jsonl")).unwrap(),
            events_before
        );
    }

    #[test]
    fn test_preview_policy_statuses_remain_distinct_and_ineligible() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        storage.init().unwrap();
        fs::write(repo.path().join("root.csv"), "a,b").unwrap();
        let executor = CommandExecutor::new(storage.clone());
        let unconfigured = executor.preview_archive_document("root.csv").unwrap();
        assert_eq!(
            unconfigured.policy_status(),
            crate::domain::artifact_plan::PolicyStatus::Unconfigured
        );
        assert!(!unconfigured.eligible());
        assert!(unconfigured
            .blockers()
            .iter()
            .any(|b| b.code == BlockerCode::PolicyUnconfigured));

        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\".\"]\n",
        )
        .unwrap();
        let incomplete = CommandExecutor::new(storage)
            .preview_archive_document("root.csv")
            .unwrap();
        assert_eq!(
            incomplete.policy_status(),
            crate::domain::artifact_plan::PolicyStatus::Incomplete
        );
        assert!(!incomplete.eligible());
        assert!(incomplete
            .blockers()
            .iter()
            .any(|b| b.code == BlockerCode::PolicyIncomplete));
    }

    #[test]
    fn test_preview_every_partial_policy_is_incomplete_and_explicit_empty_is_configured() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        storage.init().unwrap();
        fs::write(repo.path().join("root.csv"), "a,b").unwrap();
        let fields = [
            "managed_paths = []",
            "permanent_paths = []",
            "archive_root = \"\"",
        ];

        for mask in 0..7 {
            let authored = fields
                .iter()
                .enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, field)| *field)
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(
                storage.root().join("config.toml"),
                format!("[documentation]\n{authored}\n"),
            )
            .unwrap();
            let plan = CommandExecutor::new(storage.clone())
                .preview_archive_document("root.csv")
                .unwrap();
            assert_eq!(
                plan.policy_status(),
                crate::domain::artifact_plan::PolicyStatus::Incomplete,
                "mask {mask} must not receive accessor defaults"
            );
            assert!(!plan.eligible());
        }

        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = []\npermanent_paths = []\narchive_root = \"\"\n",
        )
        .unwrap();
        let configured = CommandExecutor::new(storage)
            .preview_archive_document("root.csv")
            .unwrap();
        assert_eq!(
            configured.policy_status(),
            crate::domain::artifact_plan::PolicyStatus::Configured
        );
        assert!(!configured.blockers().iter().any(|blocker| matches!(
            blocker.code,
            BlockerCode::PolicyIncomplete | BlockerCode::PolicyUnconfigured
        )));
    }

    #[test]
    fn test_preview_non_terminal_container_and_foreign_destination_are_blocked() {
        let (repo, executor, id) = configured_repo();
        let mut issue = executor.storage.load_issue(&id).unwrap();
        issue.state = State::InProgress;
        executor.storage.save_issue(issue).unwrap();
        let destination = repo.path().join("archive").join(&id[..8]);
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join(".jit-container"), "foreign-container-id\n").unwrap();

        let plan = executor.preview_archive_container(&id[..8]).unwrap();
        assert!(!plan.eligible());
        assert!(plan
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::NonTerminalTarget));
        assert!(plan
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::DestinationConflict));

        fs::remove_file(destination.join(".jit-container")).unwrap();
        fs::write(destination.join("foreign.txt"), "not in the plan").unwrap();
        let markerless = executor.preview_archive_container(&id).unwrap();
        assert!(markerless
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::DestinationConflict));
    }

    #[cfg(unix)]
    #[test]
    fn test_preview_blocks_symlink_artifact_without_following_it() {
        use std::os::unix::fs::symlink;

        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        storage.init().unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
        )
        .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("real.md"), "[referent](secret.html)").unwrap();
        fs::write(repo.path().join("fixtures/secret.html"), "referent-only").unwrap();
        symlink(
            repo.path().join("real.md"),
            repo.path().join("fixtures/link.md"),
        )
        .unwrap();

        let plan = CommandExecutor::new(storage)
            .preview_archive_document("fixtures/link.md")
            .unwrap();
        let artifact = plan
            .artifacts()
            .iter()
            .find(|artifact| artifact.source() == "fixtures/link.md")
            .unwrap();
        assert_eq!(plan.artifacts().len(), 1);
        assert!(artifact.edges().is_empty());
        assert_eq!(
            artifact.action(),
            crate::domain::artifact_plan::ArtifactAction::Block
        );
        assert!(artifact
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::SymlinkArtifact));

        let regular_repo = TempDir::new().unwrap();
        let regular_storage = JsonFileStorage::new(regular_repo.path().join(".jit"));
        regular_storage.init().unwrap();
        fs::write(
            regular_storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
        )
        .unwrap();
        fs::create_dir(regular_repo.path().join("fixtures")).unwrap();
        fs::create_dir(regular_repo.path().join("real-archive")).unwrap();
        fs::write(regular_repo.path().join("fixtures/root.md"), "root").unwrap();
        symlink(
            regular_repo.path().join("real-archive"),
            regular_repo.path().join("archive"),
        )
        .unwrap();
        let destination_symlink = CommandExecutor::new(regular_storage)
            .preview_archive_document("fixtures/root.md")
            .unwrap();
        assert!(destination_symlink.artifacts()[0]
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::SymlinkArtifact));

        let owner_repo = TempDir::new().unwrap();
        let owner_storage = JsonFileStorage::new(owner_repo.path().join(".jit"));
        owner_storage.init().unwrap();
        fs::write(
            owner_storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
        )
        .unwrap();
        fs::create_dir_all(owner_repo.path().join("docs")).unwrap();
        fs::create_dir_all(owner_repo.path().join("fixtures")).unwrap();
        fs::create_dir_all(owner_repo.path().join("referents")).unwrap();
        fs::write(
            owner_repo.path().join("referents/outside.md"),
            "[selected](../fixtures/selected.md)",
        )
        .unwrap();
        fs::write(owner_repo.path().join("fixtures/selected.md"), "selected").unwrap();
        symlink(
            owner_repo.path().join("referents/outside.md"),
            owner_repo.path().join("docs/outside.md"),
        )
        .unwrap();
        let mut outside = Issue::new("Active outside owner".into(), String::new());
        outside.state = State::InProgress;
        outside.documents = vec![DocumentReference::new("docs/outside.md".into())];
        owner_storage.save_issue(outside).unwrap();

        let owner_plan = CommandExecutor::new(owner_storage)
            .preview_archive_document("fixtures/selected.md")
            .unwrap();
        let selected = &owner_plan.artifacts()[0];
        assert_eq!(
            selected.action(),
            crate::domain::artifact_plan::ArtifactAction::Move
        );
        assert!(!selected.evidence().iter().any(|evidence| matches!(
            evidence,
            crate::domain::artifact_plan::EvidenceCode::OutsideOwner
                | crate::domain::artifact_plan::EvidenceCode::ActiveOwner
        )));
    }
}
