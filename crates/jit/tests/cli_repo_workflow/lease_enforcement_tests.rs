//! The one lease rule: in `strict` mode anyone else's active lease refuses a write,
//! and no lease lets it through. Every CLI leaf is classified here, so a new
//! command cannot silently miss the check.

use clap::{CommandFactory, Parser};
use jit::cli::Cli;
use jit::domain::identity::IssueId;
use jit::repository_state::{FileMode, VirtualPath};
use jit::storage::claim_coordinator::Lease;
use jit::storage::worktree_paths::WorktreePaths;
use jit::storage::{PutFile, RedoRecord};
use jit::test_cli::JitCli;
use jit::test_lease_store::{history_path, lease_path, plant_lease};
use serde_json::Value;
use std::collections::BTreeSet;
use std::process::Output;

const OWNER: &str = "agent:owner";
const OTHER: &str = "agent:other";

/// Leaves another writer's lease refuses and that succeed with no lease, as argv
/// lines; `{x}`, `{y}`, `{z}` and `{w}` expand to the fixture's issues.
const REFUSES: &[(&str, &str)] = &[
    ("apply", "apply plan {x}"),
    ("dep add", "dep add {x} {y}"),
    ("dep rm", "dep rm {x} {z}"),
    ("doc add", "doc add {x} docs/b.md"),
    ("doc remove", "doc remove {x} docs/a.md"),
    ("claim acquire", "claim acquire {x} --ttl 60"),
    ("gate remove", "gate remove auto"),
    ("gate update", "gate update auto --title Renamed"),
    ("gate add", "gate add {x} unused"),
    ("gate evaluate", "gate evaluate {x} auto"),
    ("gate evaluate-all", "gate evaluate-all {x}"),
    ("gate evaluate-many", "gate evaluate-many auto {x}"),
    ("gate fail", "gate fail {w} manual --by agent:other"),
    ("gate preset apply", "gate preset apply seeded {x}"),
    ("issue assign", "issue assign {x} agent:other"),
    ("issue claim", "issue claim {w} agent:other"),
    (
        "issue claim-next",
        "issue claim-next agent:other --filter priority:critical",
    ),
    ("issue delete", "issue delete {x}"),
    ("issue reject", "issue reject {x} --reason wontfix"),
    ("issue unassign", "issue unassign {x}"),
    ("issue update", "issue update {x} --priority high"),
];

/// Leaves that write a leased issue by a path [`EXEMPT_PATHS`] proves.
const EXEMPT: &[&str] = &[
    "archive container",
    "archive document",
    "issue release",
    "recover",
    "validate",
];

/// Mutating leaves that write no existing issue record, run in order against one
/// repository.
const NO_RECORD_WRITERS: &[(&str, &str, &str)] = &[
    (
        "config set",
        OTHER,
        "config set validation.default_type task",
    ),
    (
        "gate define",
        OTHER,
        "gate define extra --title E --description d --stage postcheck --mode manual",
    ),
    ("gate preset create", OTHER, "gate preset create {y} second"),
    ("hooks install", OTHER, "hooks install"),
    ("init", OTHER, "init"),
    (
        "profile pack",
        OTHER,
        "profile pack --source packages/p --output p.tar",
    ),
    (
        "profile add",
        OTHER,
        "profile add --archive p.tar --destination packages/q",
    ),
    ("profile apply", OTHER, "profile apply --profile packages/p"),
    ("project render", OTHER, "project render"),
    ("snapshot export", OTHER, "snapshot export --out snapshot"),
    ("claim renew", OWNER, "claim renew {x}"),
    ("claim heartbeat", OWNER, "claim heartbeat {x}"),
    ("claim release", OTHER, "claim release {x}"),
    (
        "claim force-evict",
        OTHER,
        "claim force-evict {w} --reason crashed",
    ),
    (
        "issue batch-create",
        OTHER,
        "issue batch-create --from-json batch.json",
    ),
    ("issue create", OTHER, "issue create --title New"),
];

/// Leaves the production classifier opens no mutation session for, with arguments
/// that parse. `worktree exec` writes nothing itself; its child is the caller's.
const READ_ONLY: &[(&str, &str)] = &[
    ("archive candidates", ""),
    ("claim list", ""),
    ("claim status", ""),
    ("config get", "worktree.enforce_leases"),
    ("config show", ""),
    ("config show-hierarchy", ""),
    ("config validate", ""),
    ("doc assets list", "deadbeef a.md"),
    ("doc check-links", ""),
    ("doc conformance", ""),
    ("doc diff", "deadbeef a.md --from HEAD"),
    ("doc dir", "deadbeef dev/active"),
    ("doc history", "deadbeef a.md"),
    ("doc list", "deadbeef"),
    ("doc show", "deadbeef a.md"),
    ("events query", ""),
    ("events tail", ""),
    ("gate list", ""),
    ("gate preset list", ""),
    ("gate preset show", "seeded"),
    ("gate show", "auto"),
    ("gate status", "deadbeef"),
    ("gate status-all", "deadbeef"),
    ("graph deps", "deadbeef"),
    ("graph export", ""),
    ("graph rdeps", "deadbeef"),
    ("graph roots", ""),
    ("graph tree", ""),
    ("invariant check", ""),
    ("issue children", "deadbeef"),
    ("issue list", ""),
    ("issue progress", "deadbeef"),
    ("issue search", "text"),
    ("issue show", "deadbeef"),
    ("issue status", "deadbeef"),
    ("item list", ""),
    ("item resolve", "@/charter/D-1"),
    ("item search", "text"),
    ("item show", "@/charter/D-1"),
    ("label namespaces", ""),
    ("label values", "type"),
    ("list", ""),
    ("profile diff", "--profile packages/p"),
    ("profile show", "--profile packages/p"),
    ("query all", ""),
    ("query available", ""),
    ("query blocked", ""),
    ("query closed", ""),
    ("query count", "--by state"),
    ("query divergence", ""),
    ("query strategic", ""),
    ("rdeps", "deadbeef"),
    ("search", "text"),
    ("status", ""),
    ("version", ""),
    ("worktree exec", "-- true"),
    ("worktree info", ""),
    ("worktree list", ""),
    ("worktree store-divergence", ""),
];

/// `jit serve` opens a session but is not run here: it stays up, and its API only
/// reads (`crates/server/src/routes.rs` routes `get` alone).
const SERVER: &str = "serve";

/// Writes that reach a leased issue by design, each with its reason and proof.
const EXEMPT_PATHS: &[(&str, &str, fn())] = &[
    (
        "issue release",
        "documented timeout recovery; a required reason, and its event names who released",
        release_records_the_acting_identity_and_reason,
    ),
    (
        "issue claim on an unleased issue",
        "assignment bookkeeping; a leased issue is covered by REFUSES",
        claim_of_an_unleased_issue_writes,
    ),
    (
        "derived promotions of dependents",
        "derived from the graph; the closing agent has no lease to hold on the dependent",
        close_promotes_a_leased_dependent,
    ),
    (
        "issue delete's cascade over dependents",
        "maintains the graph invariant; the named issue itself refuses",
        delete_rewrites_a_leased_dependent,
    ),
    (
        "validate --fix derived repair",
        "readiness is derived from the graph",
        fix_repairs_readiness_of_a_leased_issue,
    ),
    (
        "validate --fix transitive reduction",
        "removes only an edge the graph already implies",
        fix_reduces_a_leased_issue,
    ),
    (
        "archive --execute",
        "targets are terminal, and it leaves a non-terminal record alone",
        archive_relinks_terminal_records_only,
    ),
    (
        "jit recover",
        "replays a mutation that already committed",
        recover_replays_a_committed_mutation,
    ),
];

struct Repo {
    cli: JitCli,
    x: String,
    y: String,
    z: String,
    w: String,
}

impl Repo {
    /// A strict repository where [`OWNER`] leases `x`, which depends on `z` and
    /// links `docs/a.md`, and `w`, ready and the only critical issue; `y` is free.
    fn new() -> Self {
        let cli = JitCli::uninitialised().unwrap();
        cli.init_git().unwrap();
        cli.run(["init"]).unwrap();
        std::fs::create_dir(cli.root().join("docs")).unwrap();
        std::fs::write(cli.root().join("docs/a.md"), "a\n").unwrap();
        std::fs::write(cli.root().join("docs/b.md"), "b\n").unwrap();
        std::fs::write(
            cli.root().join("batch.json"),
            r#"[{"key":"a","title":"Batched"}]"#,
        )
        .unwrap();
        let package = cli.root().join("packages/p/assets");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("profile.txt"), "asset\n").unwrap();
        std::fs::write(
            cli.root().join("packages/p/manifest.toml"),
            concat!(
                "[profile]\nmanifest-version = 2\nid = \"probe\"\nversion = \"1.0.0\"\n",
                "compatible-jit = \">=0.2.0, <2.0.0\"\n\n",
                "[[asset]]\nsource = \"assets/profile.txt\"\ntarget = \"docs/profile.txt\"\n",
            ),
        )
        .unwrap();
        std::fs::write(
            cli.root().join(".jit/templates.toml"),
            concat!(
                "[[template]]\nname = \"plan\"\napplies_to = [\"task\"]\n\n",
                "  [[template.anchors]]\n  name = \"container\"\n\n",
                "  [[template.nodes]]\n  role = \"planning\"\n  type = \"task\"\n",
                "  description = \"Plan {container.title}.\"\n\n",
                "  [[template.anchor_edges]]\n  from = \"container\"\n  to = \"planning\"\n",
            ),
        )
        .unwrap();
        let mut repo = Self {
            cli,
            x: String::new(),
            y: String::new(),
            z: String::new(),
            w: String::new(),
        };
        repo.append_config(
            "\n[worktree]\nenforce_leases = \"off\"\nwrite_policy = \"allow\"\n\
             \n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"docs\"]\n\
             permanent_paths = []\narchive_root = \"archive\"\n",
        );
        repo.x = repo.create("Leased");
        repo.y = repo.create("Free");
        repo.z = repo.create("Blocker");
        repo.w = repo.create("Critical");
        const GATE: &str = "--title G --description d --stage postcheck --mode";
        for line in [
            format!("gate define manual {GATE} manual"),
            format!("gate define unused {GATE} manual"),
            format!("gate define auto {GATE} auto --checker-command true"),
            format!("gate add {} manual", repo.y),
            format!("gate preset create {} seeded", repo.y),
            format!("gate add {} auto", repo.x),
            format!("gate add {} manual", repo.w),
            format!("dep add {} {}", repo.x, repo.z),
            format!("doc add {} docs/a.md", repo.x),
            format!("issue update {} --priority critical", repo.w),
            format!("issue update {} --label type:task --force", repo.x),
        ] {
            repo.owner(&line.split_whitespace().collect::<Vec<_>>());
        }
        repo.set_mode("strict");
        repo.cli.commit("seed").unwrap();
        repo.owner(&["claim", "acquire", &repo.x, "--ttl", "600"]);
        repo.owner(&["claim", "acquire", &repo.w, "--ttl", "600"]);
        repo.owner(&["issue", "unassign", &repo.x]);
        repo.owner(&["issue", "unassign", &repo.w]);
        repo
    }

    fn release_leases(&self) {
        self.owner(&["claim", "release", &self.x]);
        self.owner(&["claim", "release", &self.w]);
    }

    /// Every issue record and the event log, as they stand.
    fn snapshot(&self) -> (Vec<String>, usize) {
        let records = [&self.x, &self.y, &self.z, &self.w]
            .map(|id| self.raw_record(id))
            .to_vec();
        (records, self.events().len())
    }

    fn append_config(&self, section: &str) {
        let path = self.cli.root().join(".jit/config.toml");
        let config = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, config + section).unwrap();
    }

    fn set_mode(&self, mode: &str) {
        let path = self.cli.root().join(".jit/config.toml");
        let config = std::fs::read_to_string(&path).unwrap();
        let declared = config
            .lines()
            .map(|line| match line.starts_with("enforce_leases") {
                true => format!("enforce_leases = \"{mode}\""),
                false => line.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, declared + "\n").unwrap();
    }

    fn create(&self, title: &str) -> String {
        let output = self.run(OWNER, &["issue", "create", "--title", title, "--json"]);
        assert!(output.status.success(), "{}", stderr(&output));
        jit::test_cli::created_issue_id(&serde_json::from_slice(&output.stdout).unwrap()).unwrap()
    }

    fn run(&self, agent: &str, args: &[&str]) -> Output {
        self.cli
            .command_with_env(args, [("JIT_AGENT_ID", agent), ("JIT_ALLOW_DELETION", "1")])
            .output()
            .unwrap()
    }

    fn owner(&self, args: &[&str]) {
        let output = self.run(OWNER, args);
        assert!(output.status.success(), "{args:?}: {}", stderr(&output));
    }

    fn other(&self, args: &[&str]) -> Output {
        self.run(OTHER, args)
    }

    /// A writer with no identity: no variable, and a home with no agent config.
    fn anonymous(&self, args: &[&str]) -> Output {
        let home = self.cli.root().join(".git");
        self.cli
            .command_with_env(args, [("HOME", home.as_os_str())])
            .env("JIT_ALLOW_DELETION", "1")
            .output()
            .unwrap()
    }

    fn expand(&self, argv: &str) -> Vec<String> {
        argv.split_whitespace()
            .map(|arg| match arg {
                "{x}" => self.x.clone(),
                "{y}" => self.y.clone(),
                "{z}" => self.z.clone(),
                "{w}" => self.w.clone(),
                other => other.to_owned(),
            })
            .collect()
    }

    fn record(&self, id: &str) -> Value {
        serde_json::from_str(&self.raw_record(id)).unwrap()
    }

    fn raw_record(&self, id: &str) -> String {
        std::fs::read_to_string(
            self.cli
                .root()
                .join(".jit/issues")
                .join(format!("{id}.json")),
        )
        .unwrap()
    }

    fn write_record(&self, id: &str, record: &Value) {
        std::fs::write(
            self.cli
                .root()
                .join(".jit/issues")
                .join(format!("{id}.json")),
            serde_json::to_vec_pretty(record).unwrap(),
        )
        .unwrap();
    }

    fn events(&self) -> Vec<Value> {
        std::fs::read_to_string(self.cli.root().join(".jit/events.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        stderr(output)
    );
}

fn release_records_the_acting_identity_and_reason() {
    let repo = Repo::new();
    repo.owner(&["issue", "assign", &repo.x, OWNER]);

    assert_ok(&repo.other(&["issue", "release", &repo.x, "lease lapsed"]));

    let events = repo.events();
    let released = events
        .iter()
        .rev()
        .find(|event| event["type"] == "issue_released")
        .expect("a release event");
    assert_eq!(released["released_by"], OTHER);
    assert_eq!(released["assignee"], OWNER);
    assert_eq!(released["reason"], "lease lapsed");
}

fn claim_of_an_unleased_issue_writes() {
    let repo = Repo::new();

    assert_ok(&repo.other(&["issue", "claim", &repo.y, OTHER]));

    assert_eq!(repo.record(&repo.y)["assignee"], OTHER);
}

fn close_promotes_a_leased_dependent() {
    let repo = Repo::new();
    assert_eq!(repo.record(&repo.x)["state"], "backlog");
    assert_ok(&repo.other(&["claim", "acquire", &repo.z, "--ttl", "600"]));

    assert_ok(&repo.other(&["issue", "update", &repo.z, "--state", "done"]));

    assert_ne!(repo.record(&repo.x)["state"], "backlog");
}

fn delete_rewrites_a_leased_dependent() {
    let repo = Repo::new();
    assert_ok(&repo.other(&["claim", "acquire", &repo.z, "--ttl", "600"]));

    assert_ok(&repo.other(&["issue", "delete", &repo.z]));

    assert_eq!(repo.record(&repo.x)["dependencies"], serde_json::json!([]));
}

fn fix_repairs_readiness_of_a_leased_issue() {
    let repo = Repo::new();
    let mut record = repo.record(&repo.x);
    record["state"] = "ready".into();
    repo.write_record(&repo.x, &record);

    repo.other(&["validate", "--fix"]);

    assert_eq!(repo.record(&repo.x)["state"], "backlog");
}

fn fix_reduces_a_leased_issue() {
    let repo = Repo::new();
    repo.owner(&["dep", "add", &repo.z, &repo.y]);
    let mut record = repo.record(&repo.x);
    record["dependencies"]
        .as_array_mut()
        .unwrap()
        .push(repo.y.clone().into());
    repo.write_record(&repo.x, &record);

    repo.other(&["validate", "--fix"]);

    assert_eq!(
        repo.record(&repo.x)["dependencies"],
        serde_json::json!([repo.z])
    );
}

fn archive_relinks_terminal_records_only() {
    let repo = Repo::new();
    repo.owner(&["claim", "acquire", &repo.z, "--ttl", "600"]);
    repo.owner(&["doc", "add", &repo.z, "docs/a.md"]);
    repo.owner(&["issue", "update", &repo.z, "--state", "done"]);
    let active_sharer = repo.raw_record(&repo.x);

    assert_ok(&repo.other(&["archive", "container", &repo.z, "--execute"]));

    assert_eq!(repo.record(&repo.z)["state"], "archived");
    assert_ne!(repo.record(&repo.z)["documents"][0]["path"], "docs/a.md");
    assert_eq!(repo.raw_record(&repo.x), active_sharer);
    let refused = repo.other(&["archive", "document", "docs/a.md", "--execute"]);
    assert!(!refused.status.success(), "a non-terminal owner refuses");
    assert_eq!(repo.raw_record(&repo.x), active_sharer);
}

fn recover_replays_a_committed_mutation() {
    let repo = Repo::new();
    let mut record = repo.record(&repo.x);
    record["priority"] = "high".into();
    let id: IssueId = repo.x.parse().unwrap();
    RedoRecord::new(
        Vec::new(),
        vec![PutFile {
            path: VirtualPath::issue_record(&id),
            bytes: serde_json::to_vec_pretty(&record).unwrap(),
            mode: FileMode::Regular,
        }],
        Vec::new(),
        None,
    )
    .unwrap()
    .commit(&repo.cli.root().join(".jit"))
    .unwrap();

    assert_ok(&repo.other(&["recover"]));

    assert_eq!(repo.record(&repo.x)["priority"], "high");
}

/// The CLI's leaf commands, space-separated.
fn clap_leaves(command: &clap::Command, prefix: &str) -> Vec<String> {
    let subcommands = command
        .get_subcommands()
        .filter(|sub| sub.get_name() != "help")
        .collect::<Vec<_>>();
    if subcommands.is_empty() {
        return vec![prefix.to_owned()];
    }
    subcommands
        .into_iter()
        .flat_map(|sub| {
            let path = format!("{prefix} {}", sub.get_name());
            clap_leaves(sub, path.trim())
        })
        .collect()
}

#[test]
fn test_strict_lease_every_cli_leaf_is_classified_and_only_anothers_lease_refuses() {
    let classified = REFUSES
        .iter()
        .map(|(leaf, _)| *leaf)
        .chain(EXEMPT.iter().copied())
        .chain(NO_RECORD_WRITERS.iter().map(|(leaf, _, _)| *leaf))
        .chain(READ_ONLY.iter().map(|(leaf, _)| *leaf))
        .chain([SERVER])
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let actual = clap_leaves(&Cli::command(), "")
        .into_iter()
        .collect::<BTreeSet<_>>();
    assert_eq!(classified, actual, "classify every CLI leaf");

    for (leaf, argv) in REFUSES {
        let repo = Repo::new();
        let argv = repo.expand(argv);
        let argv = argv.iter().map(String::as_str).collect::<Vec<_>>();
        let before = repo.snapshot();

        let refused = repo.other(&argv);
        let anonymous = repo.anonymous(&argv);

        assert!(
            !refused.status.success(),
            "{leaf} must refuse another agent"
        );
        assert!(
            stderr(&refused).contains(OWNER),
            "{leaf} refused another agent for another reason: {}",
            stderr(&refused)
        );
        assert!(
            !anonymous.status.success(),
            "{leaf} must refuse no identity"
        );
        assert!(
            stderr(&anonymous).contains("JIT_AGENT_ID"),
            "{leaf} refused no identity for another reason: {}",
            stderr(&anonymous)
        );
        assert_eq!(repo.snapshot(), before, "{leaf} changed something");

        repo.release_leases();
        assert_ok(&repo.other(&argv));
    }
}

#[test]
fn test_read_only_leaves_are_those_the_production_classifier_opens_no_session_for() {
    for (leaf, args) in READ_ONLY {
        let line = format!("jit {leaf} {args}");
        let cli =
            Cli::try_parse_from(line.split_whitespace()).unwrap_or_else(|e| panic!("{line}: {e}"));
        assert!(
            !cli.command.unwrap().requires_recovery_dispatch(),
            "{leaf} is mutating"
        );
    }
}

#[test]
fn test_mutating_leaves_classified_as_no_record_leave_every_issue_record_alone() {
    let repo = Repo::new();
    repo.owner(&["dep", "add", &repo.y, &repo.z]);
    repo.owner(&["dep", "add", &repo.w, &repo.z]);
    let before = repo.snapshot().0;

    for (leaf, agent, argv) in NO_RECORD_WRITERS {
        let args = repo.expand(argv);
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        let parsed =
            Cli::try_parse_from(std::iter::once("jit").chain(args.iter().copied())).unwrap();
        assert!(
            parsed.command.unwrap().requires_recovery_dispatch(),
            "{leaf} is read-only"
        );

        assert_ok(&repo.run(agent, &args));

        assert_eq!(repo.snapshot().0, before, "{leaf} wrote an issue record");
    }
}

#[test]
fn test_strict_lease_exempt_paths_write_for_their_stated_reason() {
    for (name, reason, proof) in EXEMPT_PATHS {
        assert!(reason.len() >= 20, "{name} needs a reason");
        proof();
    }
}

#[test]
fn test_strict_lease_filtered_update_refuses_whole_operation_naming_leased_issues() {
    let repo = Repo::new();
    let free = repo.raw_record(&repo.y);

    let output = repo.other(&[
        "issue",
        "update",
        "--filter",
        "priority:normal",
        "--priority",
        "high",
    ]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains(&repo.x), "{}", stderr(&output));
    assert_eq!(repo.raw_record(&repo.y), free);
}

#[test]
fn test_strict_lease_holder_writes_its_own_leased_issue() {
    let repo = Repo::new();

    assert_ok(&repo.run(OWNER, &["dep", "rm", &repo.x, &repo.z]));
}

#[test]
fn test_strict_lease_enforced_mutation_leaves_the_claims_log_unchanged() {
    let repo = Repo::new();
    let paths = WorktreePaths::detect_for_data_root(&repo.cli.root().join(".jit"), repo.cli.root())
        .unwrap();
    let y: IssueId = repo.y.parse().unwrap();
    let acquired_at = chrono::Utc::now() - chrono::Duration::hours(1);
    plant_lease(
        &paths.shared_jit,
        &Lease {
            lease_id: "00000000-0000-4000-8000-0000000000e1".parse().unwrap(),
            issue_id: y.clone(),
            agent_id: OTHER.parse().unwrap(),
            worktree_id: "lapsed-worktree".parse().unwrap(),
            branch: None,
            ttl_secs: 60,
            acquired_at,
            expires_at: Some(acquired_at + chrono::Duration::seconds(60)),
            last_beat: acquired_at,
        },
    )
    .unwrap();
    let stored = || {
        [
            history_path(&paths.shared_jit),
            lease_path(&paths.shared_jit, &y),
        ]
        .map(|path| std::fs::read(path).unwrap())
    };
    let before = stored();

    for id in [&repo.x, &repo.y] {
        assert_ok(&repo.run(OWNER, &["issue", "update", id, "--priority", "high"]));
        assert_eq!(repo.record(id)["priority"], "high");
    }

    assert_eq!(stored(), before);
}

#[test]
fn test_strict_lease_refusal_precedes_the_external_precheck() {
    let repo = Repo::new();
    repo.owner(&[
        "gate",
        "define",
        "precheck",
        "--title",
        "P",
        "--description",
        "d",
        "--stage",
        "precheck",
        "--mode",
        "auto",
        "--checker-command",
        "touch precheck-ran",
    ]);
    repo.owner(&["gate", "add", &repo.w, "precheck"]);
    repo.owner(&["issue", "update", &repo.w, "--state", "ready"]);
    let marker = repo.cli.root().join("precheck-ran");
    let enter = ["issue", "claim", &repo.w, OTHER];

    let refused = repo.other(&enter);

    assert!(stderr(&refused).contains(OWNER), "{}", stderr(&refused));
    assert!(!marker.exists(), "the checker ran before the refusal");
    repo.release_leases();
    assert_ok(&repo.other(&enter));
    assert!(marker.exists());
}

#[test]
fn test_off_mode_another_agents_lease_refuses_nothing() {
    let repo = Repo::new();
    repo.set_mode("off");

    assert_ok(&repo.other(&["issue", "update", &repo.x, "--priority", "high"]));
}

#[test]
fn test_warn_mode_dependency_writes_warn_only_in_human_non_quiet_mode() {
    let repo = Repo::new();
    repo.set_mode("warn");

    for flags in [&[][..], &["--quiet"][..], &["--json"][..]] {
        for verb in ["add", "rm"] {
            let args = ["dep", verb, &repo.x, &repo.y]
                .into_iter()
                .chain(flags.iter().copied())
                .collect::<Vec<_>>();
            let output = repo.other(&args);
            assert_ok(&output);
            if flags.is_empty() {
                assert!(stderr(&output).contains(OWNER), "{}", stderr(&output));
            } else {
                assert!(output.stderr.is_empty(), "{}", stderr(&output));
            }
            if flags.contains(&"--json") {
                let response: Value = serde_json::from_slice(&output.stdout).unwrap();
                let collection = if verb == "add" { "added" } else { "removed" };
                assert_eq!(response[collection], serde_json::json!([repo.y]));
            }
            assert_eq!(
                repo.record(&repo.x)["dependencies"]
                    .as_array()
                    .unwrap()
                    .contains(&repo.y.clone().into()),
                verb == "add"
            );
        }
    }
}

#[test]
fn test_validate_fix_type_fix_refuses_another_agents_lease() {
    let repo = Repo::new();
    repo.owner(&["dep", "add", &repo.y, &repo.z]);
    repo.append_config(
        "\n[type_hierarchy.types]\ntask = 4\n\
         \n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
    );
    let mut record = repo.record(&repo.x);
    record["labels"] = serde_json::json!(["type:taks"]);
    repo.write_record(&repo.x, &record);
    let before = repo.raw_record(&repo.x);

    let output = repo.other(&["validate", "--fix"]);

    assert!(!output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains(OWNER), "{}", stderr(&output));
    assert_eq!(repo.raw_record(&repo.x), before);
}

#[test]
fn test_dependency_and_document_edits_record_an_event_naming_the_issue() {
    let repo = Repo::new();
    for args in [
        vec!["dep", "add", &repo.y, &repo.z],
        vec!["dep", "rm", &repo.y, &repo.z],
        vec!["doc", "add", &repo.y, "docs/b.md"],
        vec!["doc", "remove", &repo.y, "docs/b.md"],
    ] {
        let before = repo.events().len();

        assert_ok(&repo.other(&args));

        let events = repo.events();
        let added = &events[before..];
        assert!(
            added
                .iter()
                .any(|event| event["issue_id"] == repo.y.as_str()),
            "{args:?} recorded no event: {added:?}"
        );
    }
}
