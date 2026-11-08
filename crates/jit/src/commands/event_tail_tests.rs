use super::CommandExecutor;
use crate::declarations::{GateChecker, GateMode, GateStage};
use crate::domain::{
    identity::{IssueId, IssueIdPrefix},
    Priority,
};
use crate::storage::JsonFileStorage;
use crate::test_utils::{
    profile_package_fixture, setup_unsynced_test_repo_with_config, unsynced_file_executor,
};
use anyhow::Result;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

struct Fixture {
    repo: tempfile::TempDir,
    executor: CommandExecutor<JsonFileStorage>,
    issue: IssueId,
    package: PathBuf,
    template: crate::templates::GraphTemplate,
}

impl Fixture {
    fn new() -> Result<Self> {
        // The property is what a mutation appends, not crash durability.
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let (repo, executor) = setup_unsynced_test_repo_with_config(&taxonomy.config_fragment())?;
        let issue = executor
            .issues()
            .create_issue(
                "History-independent mutation".into(),
                String::new(),
                Priority::Normal,
                vec![],
                vec![],
                None,
                None,
                false,
            )?
            .issue
            .id;
        executor.gates().define_gate(
            &"check".parse()?,
            "Check".into(),
            String::new(),
            GateStage::Postcheck,
            GateMode::Auto,
            Some(GateChecker::Exec {
                command: "true".into(),
                timeout_seconds: 10,
                working_dir: None,
                env: Default::default(),
                pass_context: false,
                prompt: None,
                prompt_file: None,
            }),
            100,
        )?;
        executor.gates().add_gate(&issue, &"check".parse()?)?;
        fs::create_dir_all(repo.path().join("docs"))?;
        fs::write(repo.path().join("docs/guide.md"), "Guide\n")?;
        executor.documents().add_document_reference(
            &issue,
            "docs/guide.md",
            None,
            None,
            None,
            false,
        )?;
        fs::write(repo.path().join("docs/guide.md"), "![image](image.png)\n")?;
        fs::write(repo.path().join("docs/image.png"), "image")?;
        fs::create_dir_all(repo.path().join(".jit/config/gate-presets"))?;
        let preset = crate::gate_presets::GatePresetDefinition {
            name: "review".into(),
            description: "Review".into(),
            gates: vec![crate::gate_presets::GateTemplate {
                key: "review".parse()?,
                title: "Review".into(),
                description: String::new(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
            }],
        };
        fs::write(
            repo.path().join(".jit/config/gate-presets/review.json"),
            serde_json::to_vec(&preset)?,
        )?;
        let package = crate::test_utils::copy_package_tree(
            &profile_package_fixture("planner-asset-only"),
            &repo.path().join("vendor"),
        );
        let template = toml::from_str(&format!(
            "name = 'child'\napplies_to = ['{0}']\n\
             [[nodes]]\nrole = 'child'\ntype = '{0}'\n",
            taxonomy.default_type,
        ))?;
        Ok(Self {
            repo,
            executor,
            issue,
            package,
            template,
        })
    }

    fn apply_profile(&self) -> Result<()> {
        crate::test_utils::apply_selected_packages(
            &self.executor,
            std::slice::from_ref(&self.package),
        )?;
        Ok(())
    }

    fn initialize(&self, profiled: bool) -> Result<()> {
        let executor = unsynced_file_executor(self.repo.path())?;
        let selection = profiled
            .then(|| {
                executor
                    .profiles()
                    .select_profiles(std::slice::from_ref(&self.package), &Default::default())
            })
            .transpose()?;
        executor
            .profiles()
            .initialize_fresh_repository(self.repo.path(), selection.as_ref())?;
        Ok(())
    }
}

type Mutation = fn(&Fixture) -> Result<()>;

#[test]
fn test_mutations_append_over_unparseable_history_without_capturing_the_whole_log() {
    let cases: &[(&str, Mutation)] = &[
        ("issue", |f| {
            f.executor
                .issues()
                .assign_issue(&IssueIdPrefix::from(&f.issue), "human:reader".parse()?)?;
            Ok(())
        }),
        ("document add", |f| {
            f.executor.documents().add_document_reference(
                &f.issue,
                "docs/added.md",
                None,
                None,
                None,
                true,
            )?;
            Ok(())
        }),
        ("document remove", |f| {
            f.executor
                .documents()
                .remove_document_reference(&f.issue, "docs/guide.md")?;
            Ok(())
        }),
        ("document rescan", |f| {
            f.executor
                .documents()
                .list_document_assets(&f.issue, "docs/guide.md", true)?;
            Ok(())
        }),
        ("gate evaluation", |f| {
            f.executor.gates().check_gate(&f.issue, &"check".parse()?)?;
            Ok(())
        }),
        ("gate preset", |f| {
            f.executor
                .gates()
                .apply_gate_preset(&f.issue, "review", None, false, false, &[])?;
            Ok(())
        }),
        ("gate registry", |f| {
            f.executor.gates().define_gate(
                &"review".parse()?,
                "Review".into(),
                String::new(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
                100,
            )
        }),
        ("template", |f| {
            f.executor.templates().apply_template_with(
                &f.template,
                &f.issue,
                &Default::default(),
                false,
            )?;
            Ok(())
        }),
        ("profile apply", Fixture::apply_profile),
        ("profile refresh", Fixture::apply_profile),
        ("init", |f| f.initialize(false)),
        ("profiled init", |f| f.initialize(true)),
    ];
    // Each case owns its repository, so the cases run concurrently.
    let outcomes = std::thread::scope(|scope| {
        cases
            .iter()
            .map(|(name, mutate)| scope.spawn(move || run_case(name, *mutate)))
            .collect::<Vec<_>>()
            .into_iter()
            .map(crate::test_utils::join_reraising)
            .collect::<Vec<_>>()
    });
    let failures = outcomes.into_iter().flatten().collect::<Vec<_>>();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// One case's failure, if its mutation truncated the sparse history or appended
/// no event past it.
fn run_case(name: &str, mutate: Mutation) -> Option<String> {
    let run = || -> Result<()> {
        let fixture = Fixture::new()?;
        if name == "profile refresh" {
            fixture.apply_profile()?;
            fs::remove_file(fixture.repo.path().join("docs/profile.txt"))?;
        }
        if name == "init" {
            fs::remove_file(fixture.repo.path().join(".jit/invariants.toml"))?;
        }
        let path = fixture.repo.path().join(".jit/events.jsonl");
        let previous = fs::read(&path)?;
        let final_line = previous
            .split_inclusive(|byte| *byte == b'\n')
            .next_back()
            .unwrap();
        // A sparse malformed line exceeds bounded captures without allocating history.
        let mut log = File::create(&path)?;
        log.set_len(super::CAPTURE_MAX_BYTES + 1)?;
        log.seek(SeekFrom::End(0))?;
        log.write_all(b"\n")?;
        log.write_all(final_line)?;
        let before = log.metadata()?.len();
        drop(log);

        mutate(&fixture)?;

        let mut log = OpenOptions::new().read(true).open(path)?;
        assert!(log.metadata()?.len() >= before, "{name} truncated history");
        if name != "init" {
            log.seek(SeekFrom::Start(before))?;
            let mut appended = Vec::new();
            log.read_to_end(&mut appended)?;
            assert!(
                !crate::domain::event_log::parse_known_events(&appended)?.is_empty(),
                "{name} did not append an event"
            );
        }
        Ok(())
    };
    run().err().map(|error| format!("{name}: {error:#}"))
}
