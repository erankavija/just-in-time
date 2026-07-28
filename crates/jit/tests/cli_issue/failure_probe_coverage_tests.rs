//! Completeness and runtime conformance guard for machine-readable failure probes.

use super::failure_lever_registry::{failure_lever_registry, FailureLever, FailureLeverRegistry};
use super::failure_probe_fixture::{drive_failure_lever, RecordedFailure};
use jit::output::ErrorCode;
use std::collections::{BTreeSet, HashMap};
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RegistrationKind<'a> {
    Probe,
    Exemption(&'a str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Registration<'a> {
    path: &'a str,
    kind: RegistrationKind<'a>,
}

/// Derive executable machine-readable arms from the schema generated directly
/// from Clap's command tree. Intentionally do not inspect `Command::hidden`:
/// hidden arms remain executable and therefore remain in the coverage contract.
fn reflected_json_arm_paths() -> BTreeSet<String> {
    let schema = jit::schema::CommandSchema::generate();
    reflected_json_arm_paths_from(&schema.commands)
}

fn reflected_json_arm_paths_from(
    commands: &HashMap<String, jit::schema::Command>,
) -> BTreeSet<String> {
    fn collect(
        prefix: &str,
        commands: &HashMap<String, jit::schema::Command>,
        paths: &mut BTreeSet<String>,
    ) {
        commands.iter().for_each(|(name, command)| {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix} {name}")
            };

            // Flags in the generated schema belong directly to this command;
            // inherited/global flags live separately in `global_options`.
            // Record an executable parent such as bare `query` before walking
            // its independently executable descendants.
            if command.flags.iter().any(|flag| flag.name == "json") {
                paths.insert(path.clone());
            }

            if let Some(children) = command
                .subcommands
                .as_ref()
                .filter(|children| !children.is_empty())
            {
                collect(&path, children, paths);
            }
        });
    }

    let mut paths = BTreeSet::new();
    collect("", commands, &mut paths);
    paths
}

fn coverage_errors(declared: &BTreeSet<String>, registrations: &[Registration<'_>]) -> Vec<String> {
    let registered = registrations
        .iter()
        .map(|registration| registration.path.to_owned())
        .collect::<BTreeSet<_>>();
    let missing = declared
        .difference(&registered)
        .map(|path| format!("missing probe or exemption for declared arm `{path}`"));
    let stale = registered
        .difference(declared)
        .map(|path| format!("stale registry arm `{path}` no longer declares --json"));
    let empty_exemptions = registrations
        .iter()
        .filter(|registration| {
            matches!(registration.kind, RegistrationKind::Exemption(reason) if reason.trim().is_empty())
        })
        .map(|registration| format!("exemption for `{}` has no stated reason", registration.path));

    missing.chain(stale).chain(empty_exemptions).collect()
}

fn probe_contract_errors(
    path: &str,
    has_canonical_envelope: bool,
    code: Result<ErrorCode, &str>,
) -> Vec<String> {
    let envelope_error = (!has_canonical_envelope)
        .then(|| format!("`{path}` did not render the canonical error envelope"));
    let code_error = match code {
        Ok(ErrorCode::GenericError) => Some(format!("`{path}` rendered GENERIC_ERROR")),
        Ok(_) => None,
        Err(code) => Some(format!(
            "`{path}` rendered unregistered error code `{code}`"
        )),
    };

    envelope_error.into_iter().chain(code_error).collect()
}

fn registry_registrations(registry: &FailureLeverRegistry) -> Vec<Registration<'_>> {
    registry
        .arms
        .iter()
        .map(|lever| match lever {
            FailureLever::Invocation(invocation) => Registration {
                path: &invocation.path,
                kind: RegistrationKind::Probe,
            },
            FailureLever::Exemption(exemption) => Registration {
                path: &exemption.path,
                kind: RegistrationKind::Exemption(&exemption.exemption_reason),
            },
        })
        .collect()
}

fn canonical_error_code(stdout: &[u8]) -> (bool, Result<ErrorCode, String>) {
    let value: serde_json::Value = match serde_json::from_slice(stdout) {
        Ok(value) => value,
        Err(error) => return (false, Err(format!("invalid JSON: {error}"))),
    };
    let canonical = value
        .as_object()
        .is_some_and(|root| root.len() == 1 && root.contains_key("error"))
        && value["error"].as_object().is_some_and(|error| {
            error
                .keys()
                .all(|key| matches!(key.as_str(), "code" | "message" | "details" | "suggestions"))
                && error.get("code").is_some_and(serde_json::Value::is_string)
                && error
                    .get("message")
                    .is_some_and(serde_json::Value::is_string)
                && error
                    .get("suggestions")
                    .is_none_or(serde_json::Value::is_array)
                && error
                    .get("details")
                    .is_none_or(|details| !details.is_null())
        });
    let code = value["error"]["code"]
        .as_str()
        .ok_or_else(|| "missing string error.code".to_owned())
        .and_then(|code| ErrorCode::from_str(code).map_err(|_| code.to_owned()));

    (canonical, code)
}

#[test]
fn test_failure_probe_coverage_matches_clap_arms_and_emits_typed_envelopes() {
    let declared = reflected_json_arm_paths();
    let registry = failure_lever_registry();
    let registrations = registry_registrations(&registry);
    let mut errors = coverage_errors(&declared, &registrations);
    let mut census = Vec::with_capacity(registry.arms.len());

    registry
        .arms
        .iter()
        .for_each(|lever| match drive_failure_lever(lever) {
            RecordedFailure::Exempt { path, reason } => {
                census.push(format!("{path}: exemption — {reason}"));
            }
            RecordedFailure::Invoked(failure) => {
                let (has_envelope, code) = canonical_error_code(&failure.stdout);
                let code_for_diagnostic = code
                    .as_ref()
                    .map(|code| code.as_str())
                    .unwrap_or_else(|code| code.as_str());
                census.push(format!(
                    "{}: envelope={} code={code_for_diagnostic}",
                    failure.path, has_envelope
                ));
                errors.extend(probe_contract_errors(
                    &failure.path,
                    has_envelope,
                    code.as_ref().copied().map_err(String::as_str),
                ));
                if failure.status.success() {
                    errors.push(format!("`{}` probe unexpectedly succeeded", failure.path));
                }
                if let Ok(code) = code {
                    if failure.status.code() != Some(code.exit_code().code()) {
                        errors.push(format!(
                            "`{}` exited {:?}, but {} requires {}",
                            failure.path,
                            failure.status.code(),
                            code.as_str(),
                            code.exit_code().code()
                        ));
                    }
                }
            }
        });

    assert!(
        errors.is_empty(),
        "machine-readable failure coverage failed:\n{}\n\nper-arm census:\n{}",
        errors.join("\n"),
        census.join("\n")
    );
}

#[test]
fn test_coverage_errors_reports_missing_declared_arm() {
    let declared = BTreeSet::from(["issue show".to_owned(), "issue future".to_owned()]);
    let registrations = [Registration {
        path: "issue show",
        kind: RegistrationKind::Probe,
    }];

    assert_eq!(
        coverage_errors(&declared, &registrations),
        vec!["missing probe or exemption for declared arm `issue future`".to_owned()]
    );
}

#[test]
fn test_reflected_json_arm_paths_preserves_executable_parent_and_child_arms() {
    fn command(
        has_json: bool,
        subcommands: Option<HashMap<String, jit::schema::Command>>,
    ) -> jit::schema::Command {
        jit::schema::Command {
            description: String::new(),
            hidden: false,
            aliases: Vec::new(),
            subcommands,
            args: Vec::new(),
            flags: has_json
                .then(|| jit::schema::Flag {
                    name: "json".to_owned(),
                    flag_type: "boolean".to_owned(),
                    required: false,
                    description: String::new(),
                    aliases: Vec::new(),
                })
                .into_iter()
                .collect(),
            output: None,
        }
    }

    let child = command(true, None);
    let parent = command(true, Some(HashMap::from([("child".to_owned(), child)])));
    let commands = HashMap::from([("parent".to_owned(), parent)]);

    assert_eq!(
        reflected_json_arm_paths_from(&commands),
        BTreeSet::from(["parent".to_owned(), "parent child".to_owned()])
    );
}

#[test]
fn test_coverage_errors_reports_stale_registered_arm() {
    let declared = BTreeSet::from(["issue show".to_owned()]);
    let registrations = [
        Registration {
            path: "issue show",
            kind: RegistrationKind::Probe,
        },
        Registration {
            path: "issue removed",
            kind: RegistrationKind::Probe,
        },
    ];

    assert_eq!(
        coverage_errors(&declared, &registrations),
        vec!["stale registry arm `issue removed` no longer declares --json".to_owned()]
    );
}

#[test]
fn test_probe_contract_errors_rejects_generic_code() {
    assert_eq!(
        probe_contract_errors("issue show", true, Ok(ErrorCode::GenericError)),
        vec!["`issue show` rendered GENERIC_ERROR".to_owned()]
    );
}

#[test]
fn test_coverage_errors_accepts_exemption_with_reason() {
    let declared = BTreeSet::from(["version".to_owned()]);
    let registrations = [Registration {
        path: "version",
        kind: RegistrationKind::Exemption("static build information cannot fail after dispatch"),
    }];

    assert!(coverage_errors(&declared, &registrations).is_empty());
    assert!(matches!(
        registrations[0].kind,
        RegistrationKind::Exemption(reason) if !reason.is_empty()
    ));
}

#[test]
fn test_coverage_errors_rejects_exemption_without_reason() {
    let declared = BTreeSet::from(["version".to_owned()]);
    let registrations = [Registration {
        path: "version",
        kind: RegistrationKind::Exemption(""),
    }];

    assert_eq!(
        coverage_errors(&declared, &registrations),
        vec!["exemption for `version` has no stated reason".to_owned()]
    );
}
