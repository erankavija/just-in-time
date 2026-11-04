use crate::commands::CommandExecutor;
use crate::declarations::{GateChecker, GateMode, GateStage};
use crate::domain::identity::{GateKey, IssueId};
use crate::domain::GateRunResult;
use crate::storage::{IssueStore, JsonFileStorage};
use std::collections::HashMap;

pub(crate) fn seed_gate_issue(
    executor: &CommandExecutor<JsonFileStorage>,
    taxonomy: &crate::test_taxonomy::TestTaxonomy,
    gate_key: &GateKey,
    issue_title: &str,
    issue_description: &str,
) -> IssueId {
    let issue_id = executor
        .issues()
        .create_issue(
            issue_title.to_string(),
            issue_description.to_string(),
            crate::domain::Priority::Normal,
            Vec::new(),
            vec![format!("type:{}", taxonomy.type_at_level(4))],
            None,
            None,
            false,
        )
        .unwrap()
        .issue
        .id;
    executor.gates().add_gate(&issue_id, gate_key).unwrap();
    issue_id
}

pub(crate) struct ReuseFixture {
    pub(crate) repo: tempfile::TempDir,
    pub(crate) executor: CommandExecutor<JsonFileStorage>,
    pub(crate) gate_key: GateKey,
}

impl ReuseFixture {
    pub(crate) fn seed(&self, declaration: Option<(&[&str], &[&str])>, verdict_exit: u8) {
        std::fs::create_dir_all(self.repo.path().join("workspace/src")).expect("create workspace");
        std::fs::write(
            self.repo.path().join("workspace/src/lib.rs"),
            "fn main() {}\n",
        )
        .expect("write source");
        std::fs::write(self.repo.path().join("elsewhere.txt"), "unread\n").expect("write outsider");

        let log = self.repo.path().join("executions.log");
        let mut registry = self
            .executor
            .storage
            .load_gate_registry()
            .expect("the seeded registry");
        registry.gates.insert(
            self.gate_key.clone(),
            crate::declarations::GateDefinition {
                version: 1,
                key: self.gate_key.clone(),
                title: "Workspace check".to_string(),
                description: "Workspace check".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: format!(
                        "printf x >> '{}'; exit {verdict_exit}",
                        log.to_string_lossy()
                    ),
                    timeout_seconds: 30,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                inputs: declaration.map(|(roots, exclude)| {
                    crate::domain::repository_inputs::RepositoryInputs::parse(roots, exclude)
                        .expect("a declarable input set")
                }),
                priority: 100,
                reserved: HashMap::new(),
            },
        );
        let bytes = crate::declarations::serialize_gate_registry(&registry)
            .expect("the registry serializes");
        std::fs::write(self.repo.path().join(".jit/gates.toml"), bytes).expect("write registry");
    }

    pub(crate) fn issue(&self, title: &str, description: &str) -> IssueId {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        seed_gate_issue(
            &self.executor,
            &taxonomy,
            &self.gate_key,
            title,
            description,
        )
    }

    pub(crate) fn evaluate(&self, issue_id: &IssueId) -> GateRunResult {
        self.executor
            .gates()
            .check_gate(issue_id, &self.gate_key)
            .expect("the gate evaluates")
    }

    pub(crate) fn executions(&self) -> usize {
        std::fs::read(self.repo.path().join("executions.log"))
            .map(|bytes| bytes.len())
            .unwrap_or(0)
    }

    pub(crate) fn write(&self, relative: &str, contents: &str) {
        let path = self.repo.path().join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, contents).expect("write file");
    }

    pub(crate) fn remove(&self, relative: &str) {
        std::fs::remove_file(self.repo.path().join(relative)).expect("remove file");
    }

    /// Replace the gate's checker command, leaving its declared inputs and
    /// every repository file alone.
    pub(crate) fn rewrite_checker_command(&self, command: &str) {
        let mut registry = self
            .executor
            .storage
            .load_gate_registry()
            .expect("the registry loads");
        let gate = registry
            .gates
            .get_mut(&self.gate_key)
            .expect("the gate under test is registered");
        gate.checker = Some(GateChecker::Exec {
            command: command.to_string(),
            timeout_seconds: 30,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        });
        let bytes = crate::declarations::serialize_gate_registry(&registry)
            .expect("the registry serializes");
        std::fs::write(self.repo.path().join(".jit/gates.toml"), bytes).expect("write registry");
    }
}
