//! Owns the list-envelope `count` contract: every command whose generated
//! output schema carries `count` beside a collection emits a `count` equal to
//! that collection's length.

use jit::schema::{Command, CommandSchema, OutputSchema};
use jit::test_cli::JitCli;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

const GATE_KEY: &str = "envelope-gate";
const PACKAGE_SOURCE: &str = "tests/fixtures/profile-packages/planner-asset-only";
const PACKAGE: &str = "packages/envelope";
const ARCHIVE: &str = "envelope.tar";
const DOCUMENT: &str = "design.md";
const AGENT: &str = "agent:envelope";

/// One `--json` invocation of a schema command and the key its collection is
/// emitted under.
struct Invocation {
    command: String,
    args: Vec<String>,
    collection: &'static str,
}

/// Ids and paths a seeded repository offers the invocations.
struct Seeded {
    /// Requires `GATE_KEY` and depends on `dependency`.
    container: String,
    dependency: String,
}

fn invocation(command: &str, args: &[&str], collection: &'static str) -> Invocation {
    invocation_via(command, command, args, collection)
}

/// An invocation of `command` through `path`, which is not its schema path
/// (an alias, for instance).
fn invocation_via(
    command: &str,
    path: &str,
    args: &[&str],
    collection: &'static str,
) -> Invocation {
    Invocation {
        command: command.to_string(),
        args: path
            .split(' ')
            .chain(args.iter().copied())
            .map(str::to_string)
            .collect(),
        collection,
    }
}

fn invocations(seeded: &Seeded) -> Vec<Invocation> {
    let (container, dependency) = (seeded.container.as_str(), seeded.dependency.as_str());
    vec![
        invocation("issue list", &[], "issues"),
        invocation("list", &[], "issues"),
        invocation("query", &[], "issues"),
        invocation("query all", &[], "issues"),
        invocation("query available", &[], "issues"),
        invocation_via("query available", "query ready", &[], "issues"),
        invocation("query blocked", &[], "issues"),
        invocation("query closed", &[], "issues"),
        invocation("query strategic", &[], "issues"),
        invocation("query count", &["--by", "state"], "by_state"),
        invocation("query divergence", &[], "divergences"),
        invocation("issue search", &["authentication"], "issues"),
        invocation("search", &["authentication"], "results"),
        invocation("issue show", &[container, dependency], "issues"),
        invocation("issue status", &[container, dependency], "issues"),
        invocation("issue children", &[container], "issues"),
        invocation("issue progress", &[container], "by_state"),
        invocation("gate list", &[], "gates"),
        invocation("gate preset list", &[], "presets"),
        invocation("gate status", &[container, "--all"], "results"),
        invocation("gate status-all", &[container], "gates"),
        invocation(
            "gate evaluate-many",
            &[GATE_KEY, container, dependency, "--by", "human:envelope"],
            "results",
        ),
        invocation("events tail", &[], "events"),
        invocation("events query", &[], "events"),
        invocation("doc list", &[container], "documents"),
        invocation("doc assets list", &[container, DOCUMENT], "assets"),
        invocation("doc conformance", &[], "artifacts"),
        invocation("claim list", &[], "leases"),
        invocation("claim status", &[], "leases"),
        invocation("label namespaces", &[], "namespaces"),
        invocation("label values", &["type"], "values"),
        invocation("item list", &[], "items"),
        invocation("item search", &["task"], "items"),
        invocation("worktree list", &[], "worktrees"),
        invocation("worktree store-divergence", &[], "divergences"),
        invocation("graph roots", &[], "roots"),
        invocation("graph rdeps", &[dependency], "dependents"),
        invocation("rdeps", &[dependency], "dependents"),
        invocation("graph deps", &[container], "nodes"),
        invocation("graph tree", &[], "nodes"),
        invocation("invariant check", &[], "findings"),
        invocation("project render", &[], "projections"),
        invocation("archive candidates", &[], "candidates"),
        invocation("archive container", &[container], "artifacts"),
        invocation("archive document", &[DOCUMENT], "artifacts"),
        invocation("profile show", &["--profile", PACKAGE], "profiles"),
        invocation("profile diff", &["--profile", PACKAGE], "profiles"),
        invocation(
            "profile apply",
            &["--profile", PACKAGE, "--dry-run"],
            "profiles",
        ),
        invocation(
            "profile pack",
            &["--source", PACKAGE, "--output", "again.tar", "--dry-run"],
            "profiles",
        ),
        invocation(
            "profile add",
            &[
                "--archive",
                ARCHIVE,
                "--destination",
                "packages/added",
                "--dry-run",
            ],
            "profiles",
        ),
    ]
}

/// Give `cli`, a git-backed initialised repository with one commit, material
/// for every invocation: two dependent issues, a linked document, a required
/// gate, and a profile package with its archive.
fn seed_repository(cli: &JitCli) -> Seeded {
    let dependency = cli
        .create_issue([
            "--title",
            "Second authentication task",
            "--label",
            "type:task",
        ])
        .unwrap();
    let container = cli
        .create_issue([
            "--title",
            "First authentication task",
            "--description",
            "Body about auth",
            "--label",
            "type:task",
        ])
        .unwrap();
    cli.run(["dep", "add", &container, &dependency]).unwrap();
    fs::write(cli.root().join(DOCUMENT), "# Design\n\nBody.\n").unwrap();
    cli.run(["doc", "add", &container, DOCUMENT, "--doc-type", "design"])
        .unwrap();
    cli.run([
        "gate",
        "define",
        GATE_KEY,
        "--title",
        "Envelope gate",
        "--description",
        "Gate for the envelope contract",
    ])
    .unwrap();
    cli.run(["gate", "add", &container, GATE_KEY]).unwrap();
    jit::test_utils::copy_package_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join(PACKAGE_SOURCE),
        &cli.root().join(PACKAGE),
    );
    cli.run(["profile", "pack", "--source", PACKAGE, "--output", ARCHIVE])
        .unwrap();
    Seeded {
        container,
        dependency,
    }
}

fn is_array_schema(schema: &Value) -> bool {
    match &schema["type"] {
        Value::String(kind) => kind == "array",
        Value::Array(kinds) => kinds.iter().any(|kind| kind == "array"),
        _ => false,
    }
}

/// Whether an output schema, or one arm of its `oneOf`/`anyOf`/`allOf`,
/// declares `count` beside an array-typed property.
fn carries_count_beside_collection(schema: &Value) -> bool {
    let declares_both = schema["properties"].as_object().is_some_and(|properties| {
        properties.contains_key("count")
            && properties
                .iter()
                .any(|(name, property)| name != "count" && is_array_schema(property))
    });
    declares_both
        || ["oneOf", "anyOf", "allOf"].iter().any(|combinator| {
            schema[combinator]
                .as_array()
                .is_some_and(|arms| arms.iter().any(carries_count_beside_collection))
        })
}

/// The space-joined paths of every command in `commands` and below whose
/// output schema carries `count` beside a collection.
fn list_emitting_commands(commands: &HashMap<String, Command>, prefix: &str) -> BTreeSet<String> {
    commands
        .iter()
        .flat_map(|(name, command)| {
            let path = format!("{prefix}{name}");
            let own = command
                .output
                .as_ref()
                .and_then(|output| output.success_schema.as_ref())
                .is_some_and(carries_count_beside_collection)
                .then(|| path.clone());
            let nested = command
                .subcommands
                .as_ref()
                .map(|subcommands| list_emitting_commands(subcommands, &format!("{path} ")))
                .unwrap_or_default();
            own.into_iter().chain(nested)
        })
        .collect()
}

fn command_output<'a>(schema: &'a CommandSchema, path: &str) -> Option<&'a OutputSchema> {
    let mut segments = path.split(' ');
    let first = schema.commands.get(segments.next()?)?;
    segments
        .try_fold(first, |command, segment| {
            command.subcommands.as_ref()?.get(segment)
        })?
        .output
        .as_ref()
}

/// Fails naming each schema list-emitting command that has no invocation.
fn ensure_every_command_invoked(
    emitting: &BTreeSet<String>,
    invocations: &[Invocation],
) -> Result<(), String> {
    let invoked: BTreeSet<&str> = invocations.iter().map(|i| i.command.as_str()).collect();
    let missing: Vec<&str> = emitting
        .iter()
        .map(String::as_str)
        .filter(|command| !invoked.contains(command))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "list-emitting commands without an invocation: {missing:?}"
        ))
    }
}

/// Fails naming `command` when `body` carries `count` beside an array while
/// the command's output schema is the untyped fallback.
fn ensure_typed_list_schema(
    command: &str,
    body: &Value,
    output: Option<&OutputSchema>,
) -> Result<(), String> {
    let carries_list = body
        .as_object()
        .is_some_and(|fields| fields.contains_key("count") && fields.values().any(Value::is_array));
    let typed = output.is_some_and(|output| output.success_schema.is_some());
    if carries_list && !typed {
        Err(format!(
            "`{command}` emits `count` beside an array but its output schema is the untyped fallback"
        ))
    } else {
        Ok(())
    }
}

/// Fails naming `command` unless `count` is the collection's length.
fn ensure_count_matches_collection(
    command: &str,
    body: &Value,
    collection: &str,
) -> Result<(), String> {
    let count = body["count"]
        .as_u64()
        .ok_or_else(|| format!("`{command}` has no numeric `count`: {body}"))?;
    let items = body[collection]
        .as_array()
        .ok_or_else(|| format!("`{command}` has no `{collection}` array: {body}"))?;
    if usize::try_from(count) == Ok(items.len()) {
        Ok(())
    } else {
        Err(format!(
            "`{command}` count {count} differs from `{collection}` length {}",
            items.len()
        ))
    }
}

#[test]
fn test_list_envelope_count_equals_collection_length_for_every_list_emitting_command() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();
    cli.commit("base").unwrap();
    let seeded = seed_repository(&cli);
    let schema = CommandSchema::generate();
    let invocations = invocations(&seeded);

    ensure_every_command_invoked(&list_emitting_commands(&schema.commands, ""), &invocations)
        .unwrap_or_else(|failure| panic!("{failure}"));

    invocations.iter().for_each(|invocation| {
        let args = invocation.args.iter().map(String::as_str).chain(["--json"]);
        let output = cli
            .command_with_env(args, [("JIT_AGENT_ID", AGENT)])
            .output()
            .unwrap();
        let body: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "`{}` did not emit one JSON document ({error}); stderr: {}",
                invocation.command,
                String::from_utf8_lossy(&output.stderr)
            )
        });
        // A findings outcome carries its list report in `error.details`.
        let body = body["error"].get("details").cloned().unwrap_or(body);
        ensure_typed_list_schema(
            &invocation.command,
            &body,
            command_output(&schema, &invocation.command),
        )
        .and_then(|()| {
            ensure_count_matches_collection(&invocation.command, &body, invocation.collection)
        })
        .unwrap_or_else(|failure| panic!("{failure}"));
    });
}

#[test]
fn test_carries_count_beside_collection_requires_both_in_one_object_or_arm() {
    let object = |properties: Value| json!({ "type": "object", "properties": properties });
    let counted = object(json!({ "count": { "type": "integer" }, "rows": { "type": "array" } }));
    let nullable = object(json!({
        "count": { "type": "integer" },
        "rows": { "type": ["array", "null"] }
    }));
    let uncounted = object(json!({ "rows": { "type": "array" } }));
    let scalar = object(json!({ "count": { "type": "integer" }, "title": { "type": "string" } }));

    assert!(carries_count_beside_collection(&counted));
    assert!(carries_count_beside_collection(&nullable));
    assert!(carries_count_beside_collection(
        &json!({ "oneOf": [scalar.clone(), counted] })
    ));
    assert!(!carries_count_beside_collection(&uncounted));
    assert!(!carries_count_beside_collection(&scalar));
}

#[test]
fn test_ensure_every_command_invoked_names_a_command_without_an_invocation() {
    let emitting: BTreeSet<String> = ["issue list", "wave replay"].map(String::from).into();
    let covered = [invocation("issue list", &[], "issues")];

    let failure = ensure_every_command_invoked(&emitting, &covered).unwrap_err();

    assert!(failure.contains("wave replay"), "{failure}");
    assert!(!failure.contains("issue list"), "{failure}");
    assert!(ensure_every_command_invoked(&emitting, &[]).is_err());
    assert_eq!(
        ensure_every_command_invoked(
            &emitting,
            &[
                invocation("issue list", &[], "issues"),
                invocation("wave replay", &[], "waves")
            ]
        ),
        Ok(())
    );
}

#[test]
fn test_ensure_typed_list_schema_names_a_command_with_untyped_output() {
    let untyped = OutputSchema {
        success_schema: None,
        success: "CommandResponse".to_string(),
        error: "ErrorResponse".to_string(),
    };
    let typed = OutputSchema {
        success_schema: Some(json!({ "type": "object" })),
        ..untyped_clone(&untyped)
    };
    let list_body = json!({ "count": 1, "rows": [1] });

    let failure = ensure_typed_list_schema("wave replay", &list_body, Some(&untyped)).unwrap_err();

    assert!(failure.contains("wave replay"), "{failure}");
    assert!(ensure_typed_list_schema("wave replay", &list_body, None).is_err());
    assert_eq!(
        ensure_typed_list_schema("wave replay", &list_body, Some(&typed)),
        Ok(())
    );
    assert_eq!(
        ensure_typed_list_schema("wave replay", &json!({ "count": 1 }), Some(&untyped)),
        Ok(())
    );
}

#[test]
fn test_ensure_count_matches_collection_names_a_command_whose_count_differs() {
    let failure =
        ensure_count_matches_collection("wave replay", &json!({ "count": 2, "rows": [1] }), "rows")
            .unwrap_err();

    assert!(failure.contains("wave replay"), "{failure}");
    assert_eq!(
        ensure_count_matches_collection("wave replay", &json!({ "count": 1, "rows": [1] }), "rows"),
        Ok(())
    );
}

fn untyped_clone(output: &OutputSchema) -> OutputSchema {
    OutputSchema {
        success_schema: output.success_schema.clone(),
        success: output.success.clone(),
        error: output.error.clone(),
    }
}
