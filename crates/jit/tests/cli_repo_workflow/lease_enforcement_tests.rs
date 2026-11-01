//! What another agent's active lease stops in `strict` mode: every CLI leaf is
//! classified here, so a new command cannot silently miss the check.

use clap::CommandFactory;
use jit::cli::Cli;
use jit::domain::identity::IssueId;
use jit::repository_state::{FileMode, VirtualPath};
use jit::storage::{PutFile, RedoRecord};
use jit::test_cli::JitCli;
use serde_json::Value;
use std::collections::BTreeSet;
use std::process::Output;

const OWNER: &str = "agent:owner";
const OTHER: &str = "agent:other";

/// How a CLI leaf relates to an issue record another agent has leased.
enum Coverage {
    /// Refuses; `{x}`, `{y}` and `{z}` expand to the fixture's issues.
    Refuses(&'static [&'static str]),
    /// Writes the record by a reason [`EXEMPT_PATHS`] proves.
    Exempt,
    /// Writes no existing issue record.
    NoRecord,
}

use Coverage::{Exempt, NoRecord, Refuses};

const LEAVES: &[(&str, Coverage)] = &[
    ("apply", Refuses(&["apply", "plan", "{x}"])),
    ("archive candidates", NoRecord),
    ("archive container", Exempt),
    ("archive document", Exempt),
    ("claim acquire", NoRecord),
    ("claim force-evict", NoRecord),
    ("claim heartbeat", NoRecord),
    ("claim list", NoRecord),
    ("claim release", NoRecord),
    ("claim renew", NoRecord),
    ("claim status", NoRecord),
    ("config get", NoRecord),
    ("config set", NoRecord),
    ("config show", NoRecord),
    ("config show-hierarchy", NoRecord),
    ("config validate", NoRecord),
    ("dep add", Refuses(&["dep", "add", "{x}", "{y}"])),
    ("dep rm", Refuses(&["dep", "rm", "{x}", "{z}"])),
    ("doc add", Refuses(&["doc", "add", "{x}", "docs/b.md"])),
    ("doc assets list", NoRecord),
    ("doc check-links", NoRecord),
    ("doc conformance", NoRecord),
    ("doc diff", NoRecord),
    ("doc dir", NoRecord),
    ("doc history", NoRecord),
    ("doc list", NoRecord),
    (
        "doc remove",
        Refuses(&["doc", "remove", "{x}", "docs/a.md"]),
    ),
    ("doc show", NoRecord),
    ("events query", NoRecord),
    ("events tail", NoRecord),
    ("gate add", Refuses(&["gate", "add", "{x}", "manual"])),
    ("gate define", NoRecord),
    (
        "gate evaluate",
        Refuses(&["gate", "evaluate", "{x}", "auto"]),
    ),
    (
        "gate evaluate-all",
        Refuses(&["gate", "evaluate-all", "{x}"]),
    ),
    (
        "gate evaluate-many",
        Refuses(&["gate", "evaluate-many", "auto", "{x}"]),
    ),
    (
        "gate fail",
        Refuses(&["gate", "fail", "{x}", "auto", "--by", OTHER]),
    ),
    ("gate list", NoRecord),
    (
        "gate preset apply",
        Refuses(&["gate", "preset", "apply", "seeded", "{x}"]),
    ),
    ("gate preset create", NoRecord),
    ("gate preset list", NoRecord),
    ("gate preset show", NoRecord),
    ("gate remove", NoRecord),
    ("gate show", NoRecord),
    ("gate status", NoRecord),
    ("gate status-all", NoRecord),
    ("gate update", NoRecord),
    ("graph deps", NoRecord),
    ("graph export", NoRecord),
    ("graph rdeps", NoRecord),
    ("graph roots", NoRecord),
    ("graph tree", NoRecord),
    ("hooks install", NoRecord),
    ("init", NoRecord),
    ("invariant check", NoRecord),
    ("issue assign", Refuses(&["issue", "assign", "{x}", OTHER])),
    ("issue batch-create", NoRecord),
    ("issue children", NoRecord),
    ("issue claim", Refuses(&["issue", "claim", "{x}", OTHER])),
    (
        "issue claim-next",
        Refuses(&[
            "issue",
            "claim-next",
            OTHER,
            "--filter",
            "state:in_progress",
        ]),
    ),
    ("issue create", NoRecord),
    ("issue delete", Refuses(&["issue", "delete", "{x}"])),
    ("issue list", NoRecord),
    ("issue progress", NoRecord),
    (
        "issue reject",
        Refuses(&["issue", "reject", "{x}", "--reason", "wontfix"]),
    ),
    ("issue release", Exempt),
    ("issue search", NoRecord),
    ("issue show", NoRecord),
    ("issue status", NoRecord),
    ("issue unassign", Refuses(&["issue", "unassign", "{x}"])),
    (
        "issue update",
        Refuses(&["issue", "update", "{x}", "--priority", "high"]),
    ),
    ("item list", NoRecord),
    ("item resolve", NoRecord),
    ("item search", NoRecord),
    ("item show", NoRecord),
    ("label namespaces", NoRecord),
    ("label values", NoRecord),
    ("list", NoRecord),
    ("profile add", NoRecord),
    ("profile apply", NoRecord),
    ("profile diff", NoRecord),
    ("profile pack", NoRecord),
    ("profile show", NoRecord),
    ("project render", NoRecord),
    ("query all", NoRecord),
    ("query available", NoRecord),
    ("query blocked", NoRecord),
    ("query closed", NoRecord),
    ("query count", NoRecord),
    ("query divergence", NoRecord),
    ("query strategic", NoRecord),
    ("rdeps", NoRecord),
    ("recover", Exempt),
    ("search", NoRecord),
    ("serve", NoRecord),
    ("snapshot export", NoRecord),
    ("status", NoRecord),
    ("validate", Exempt),
    ("version", NoRecord),
    ("worktree exec", NoRecord),
    ("worktree info", NoRecord),
    ("worktree list", NoRecord),
    ("worktree store-divergence", NoRecord),
];

/// Writes that reach a leased issue by design, each with its reason and proof.
const EXEMPT_PATHS: &[(&str, &str, fn())] = &[
    (
        "issue release",
        "documented timeout recovery; a required reason, and its event names who released",
        release_records_the_acting_identity_and_reason,
    ),
    (
        "issue claim on an unleased issue",
        "assignment bookkeeping; a leased issue is covered by the Refuses row",
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
}

impl Repo {
    /// A strict repository where [`OWNER`] leases `x`, which depends on `z` and
    /// links `docs/a.md`; `y` is free.
    fn new() -> Self {
        let cli = JitCli::uninitialised().unwrap();
        cli.init_git().unwrap();
        cli.run(["init"]).unwrap();
        std::fs::create_dir(cli.root().join("docs")).unwrap();
        std::fs::write(cli.root().join("docs/a.md"), "a\n").unwrap();
        std::fs::write(cli.root().join("docs/b.md"), "b\n").unwrap();
        std::fs::write(
            cli.root().join(".jit/templates.toml"),
            concat!(
                "[[template]]\nname = \"plan\"\napplies_to = [\"task\"]\n\n",
                "  [[template.anchors]]\n  name = \"container\"\n\n",
                "  [[template.nodes]]\n  role = \"planning\"\n  type = \"task\"\n",
                "  description = \"Plan {container.title}.\"\n",
            ),
        )
        .unwrap();
        let mut repo = Self {
            cli,
            x: String::new(),
            y: String::new(),
            z: String::new(),
        };
        repo.append_config(
            "\n[worktree]\nenforce_leases = \"off\"\nwrite_policy = \"allow\"\n\
             \n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"docs\"]\n\
             permanent_paths = []\narchive_root = \"archive\"\n",
        );
        repo.x = repo.create("Leased");
        repo.y = repo.create("Free");
        repo.z = repo.create("Blocker");
        for args in [
            vec![
                "gate",
                "define",
                "manual",
                "--title",
                "M",
                "--description",
                "d",
                "--stage",
                "postcheck",
                "--mode",
                "manual",
            ],
            vec![
                "gate",
                "define",
                "auto",
                "--title",
                "A",
                "--description",
                "d",
                "--stage",
                "postcheck",
                "--mode",
                "auto",
                "--checker-command",
                "true",
            ],
            vec!["gate", "add", &repo.y, "manual"],
            vec!["gate", "preset", "create", &repo.y, "seeded"],
            vec!["gate", "add", &repo.x, "auto"],
            vec!["dep", "add", &repo.x, &repo.z],
            vec!["doc", "add", &repo.x, "docs/a.md"],
        ] {
            repo.owner(&args);
        }
        repo.make_strict();
        repo.cli.commit("seed").unwrap();
        repo.owner(&["claim", "acquire", &repo.x, "--ttl", "600"]);
        repo
    }

    fn append_config(&self, section: &str) {
        let path = self.cli.root().join(".jit/config.toml");
        let config = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, config + section).unwrap();
    }

    fn make_strict(&self) {
        let path = self.cli.root().join(".jit/config.toml");
        let config = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            path,
            config.replace("enforce_leases = \"off\"", "enforce_leases = \"strict\""),
        )
        .unwrap();
    }

    fn create(&self, title: &str) -> String {
        let output = self.run(
            OWNER,
            &["issue", "create", "--title", title, "--orphan", "--json"],
        );
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

    fn expand(&self, argv: &[&str]) -> Vec<String> {
        argv.iter()
            .map(|arg| match *arg {
                "{x}" => self.x.clone(),
                "{y}" => self.y.clone(),
                "{z}" => self.z.clone(),
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
fn test_strict_lease_every_cli_leaf_is_classified_and_refusals_change_nothing() {
    let classified = LEAVES
        .iter()
        .map(|(leaf, _)| (*leaf).to_owned())
        .collect::<BTreeSet<_>>();
    let actual = clap_leaves(&Cli::command(), "")
        .into_iter()
        .collect::<BTreeSet<_>>();
    assert_eq!(classified, actual, "classify every CLI leaf in LEAVES");

    for (leaf, coverage) in LEAVES {
        let Refuses(argv) = coverage else { continue };
        let repo = Repo::new();
        let before = repo.raw_record(&repo.x);
        let events_before = repo.events().len();

        let output = repo.other(
            &repo
                .expand(argv)
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        );

        assert!(!output.status.success(), "{leaf} must refuse");
        assert_eq!(repo.raw_record(&repo.x), before, "{leaf} wrote the record");
        assert_eq!(
            repo.events().len(),
            events_before,
            "{leaf} appended an event"
        );
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
fn test_strict_lease_new_checks_allow_an_unleased_issue_and_the_holder() {
    let repo = Repo::new();

    assert_ok(&repo.other(&["dep", "add", &repo.y, &repo.z]));
    assert_ok(&repo.other(&["doc", "add", &repo.y, "docs/b.md"]));
    assert_ok(&repo.run(OWNER, &["dep", "rm", &repo.x, &repo.z]));
    assert_ok(&repo.run(OWNER, &["doc", "remove", &repo.x, "docs/a.md"]));
}

#[test]
fn test_warn_mode_dep_add_warns_and_writes() {
    let repo = Repo::new();
    let path = repo.cli.root().join(".jit/config.toml");
    let config = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        config.replace("enforce_leases = \"strict\"", "enforce_leases = \"warn\""),
    )
    .unwrap();

    let output = repo.other(&["dep", "add", &repo.x, &repo.y]);

    assert_ok(&output);
    assert!(stderr(&output).contains(OWNER), "{}", stderr(&output));
    assert!(repo.record(&repo.x)["dependencies"]
        .as_array()
        .unwrap()
        .contains(&repo.y.clone().into()));
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
