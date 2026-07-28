//! Completeness and runtime conformance guard for machine-readable failure probes.

use super::failure_lever_registry::{failure_lever_registry, FailureLever, FailureLeverRegistry};
use super::failure_probe_fixture::{drive_failure_lever, RecordedFailure};
use jit::output::ErrorCode;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

const CENSUS_FILE_NAME: &str = "failure-probe-census.json";
static NEXT_CENSUS_STAGING_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct FailureProbeCensus {
    schema: CensusSchema,
    arms: Vec<CensusRow>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
enum CensusSchema {
    #[serde(rename = "failure-probe-census/v1")]
    V1,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CensusRow {
    Probe {
        path: String,
        envelope_rendered: bool,
        error_code: String,
        observed_exit: i32,
    },
    Exemption {
        path: String,
        reason: String,
    },
}

impl CensusRow {
    fn path(&self) -> &str {
        match self {
            Self::Probe { path, .. } | Self::Exemption { path, .. } => path,
        }
    }
}

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

/// Locate the profile output that contains this test executable. Cargo places
/// integration-test executables in `<target>/<profile>/deps`, including when a
/// caller selects a custom target directory.
fn census_report_path() -> io::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    census_report_path_from_executable(&executable)
}

fn census_report_path_from_executable(executable: &Path) -> io::Result<PathBuf> {
    let profile_output = executable
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::other("test executable has no Cargo profile output parent"))?;
    Ok(profile_output
        .join("jit-conformance")
        .join(CENSUS_FILE_NAME))
}

fn write_census_report_to(path: &Path, report: &FailureProbeCensus) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("census report path has no parent directory"))?;
    fs::create_dir_all(directory)?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("census report path has no UTF-8 file name"))?;
    let staging_id = NEXT_CENSUS_STAGING_ID.fetch_add(1, Ordering::Relaxed);
    let staging_path = directory.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        staging_id
    ));

    let publish = (|| {
        let mut staging = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging_path)?;
        serde_json::to_writer_pretty(&mut staging, report)?;
        staging.write_all(b"\n")?;
        staging.sync_all()?;
        drop(staging);
        fs::rename(&staging_path, path)
    })();

    if publish.is_err() {
        let _ = fs::remove_file(&staging_path);
    }
    publish
}

fn write_census_report(report: &FailureProbeCensus) -> io::Result<PathBuf> {
    let path = census_report_path()?;
    write_census_report_to(&path, report)?;
    Ok(path)
}

#[test]
fn test_failure_probe_coverage_matches_clap_arms_and_emits_typed_envelopes() {
    let declared = reflected_json_arm_paths();
    let registry = failure_lever_registry();
    let registrations = registry_registrations(&registry);
    let mut errors = coverage_errors(&declared, &registrations);
    let mut diagnostics = Vec::with_capacity(registry.arms.len());
    let mut census = BTreeMap::new();

    registry
        .arms
        .iter()
        .for_each(|lever| match drive_failure_lever(lever) {
            RecordedFailure::Exempt { path, reason } => {
                diagnostics.push(format!("{path}: exemption — {reason}"));
                census
                    .entry(path.clone())
                    .or_insert(CensusRow::Exemption { path, reason });
            }
            RecordedFailure::Invoked(failure) => {
                let (has_envelope, code) = canonical_error_code(&failure.stdout);
                let code_for_diagnostic = code
                    .as_ref()
                    .map(|code| code.as_str())
                    .unwrap_or_else(|code| code.as_str());
                diagnostics.push(format!(
                    "{}: envelope={} code={code_for_diagnostic} exit={:?}",
                    failure.path,
                    has_envelope,
                    failure.status.code()
                ));
                errors.extend(probe_contract_errors(
                    &failure.path,
                    has_envelope,
                    code.as_ref().copied().map_err(String::as_str),
                ));
                if failure.status.success() {
                    errors.push(format!("`{}` probe unexpectedly succeeded", failure.path));
                }
                if let Ok(code) = &code {
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
                if let (Ok(code), Some(observed_exit)) = (code, failure.status.code()) {
                    census
                        .entry(failure.path.clone())
                        .or_insert(CensusRow::Probe {
                            path: failure.path,
                            envelope_rendered: has_envelope,
                            error_code: code.as_str().to_owned(),
                            observed_exit,
                        });
                }
            }
        });

    assert!(
        errors.is_empty(),
        "machine-readable failure coverage failed:\n{}\n\nper-arm census:\n{}",
        errors.join("\n"),
        diagnostics.join("\n")
    );

    let report = FailureProbeCensus {
        schema: CensusSchema::V1,
        arms: census.into_values().collect(),
    };
    let report_paths = report
        .arms
        .iter()
        .map(CensusRow::path)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        report_paths, declared,
        "successful census rows must equal the reflected --json arm set"
    );
    assert_eq!(
        report.arms.len(),
        declared.len(),
        "successful census must contain exactly one row per reflected arm"
    );

    let report_path = write_census_report(&report).expect("publish failure-probe census");
    let published: FailureProbeCensus = serde_json::from_slice(
        &fs::read(&report_path).expect("read published failure-probe census"),
    )
    .expect("published failure-probe census must be typed JSON");
    assert_eq!(published, report);
    published.arms.iter().for_each(|row| match row {
        CensusRow::Probe {
            envelope_rendered,
            error_code,
            observed_exit,
            ..
        } => {
            assert!(*envelope_rendered);
            let code = ErrorCode::from_str(error_code)
                .unwrap_or_else(|_| panic!("census contains unregistered code `{error_code}`"));
            assert_eq!(*observed_exit, code.exit_code().code());
        }
        CensusRow::Exemption { reason, .. } => assert!(!reason.trim().is_empty()),
    });
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

#[test]
fn test_write_census_report_replaces_stale_artifact_with_typed_rows() {
    let output = tempfile::tempdir().expect("create census output directory");
    let path = output.path().join("failure-probe-census.json");
    std::fs::write(&path, br#"{"stale":true}"#).expect("seed stale census");
    let report = FailureProbeCensus {
        schema: CensusSchema::V1,
        arms: vec![
            CensusRow::Probe {
                path: "issue show".to_owned(),
                envelope_rendered: true,
                error_code: ErrorCode::IssueNotFound.as_str().to_owned(),
                observed_exit: ErrorCode::IssueNotFound.exit_code().code(),
            },
            CensusRow::Exemption {
                path: "version".to_owned(),
                reason: "static build information cannot fail after dispatch".to_owned(),
            },
        ],
    };

    write_census_report_to(&path, &report).expect("publish census");
    let published: FailureProbeCensus =
        serde_json::from_slice(&std::fs::read(&path).expect("read published census"))
            .expect("deserialize published census");

    assert_eq!(published, report);
    assert_eq!(
        std::fs::read_dir(output.path())
            .expect("list census output directory")
            .count(),
        1,
        "atomic publication must not leave a staging file"
    );
}

#[test]
fn test_census_report_path_follows_custom_cargo_build_output() {
    let executable = Path::new("custom-cargo-target/debug/deps/cli_issue-test-hash");

    assert_eq!(
        census_report_path_from_executable(executable).expect("derive census report path"),
        Path::new("custom-cargo-target/debug/jit-conformance/failure-probe-census.json")
    );
}

#[test]
fn test_write_census_report_concurrent_publication_remains_complete_json() {
    let output = tempfile::tempdir().expect("create census output directory");
    let path = output.path().join(CENSUS_FILE_NAME);
    let report = FailureProbeCensus {
        schema: CensusSchema::V1,
        arms: vec![CensusRow::Probe {
            path: "issue show".to_owned(),
            envelope_rendered: true,
            error_code: ErrorCode::IssueNotFound.as_str().to_owned(),
            observed_exit: ErrorCode::IssueNotFound.exit_code().code(),
        }],
    };

    std::thread::scope(|scope| {
        (0..8).for_each(|_| {
            let path = path.clone();
            let report = report.clone();
            scope.spawn(move || {
                write_census_report_to(&path, &report).expect("publish concurrent census")
            });
        });
    });

    let published: FailureProbeCensus =
        serde_json::from_slice(&fs::read(&path).expect("read concurrent census"))
            .expect("concurrent publication must leave complete typed JSON");
    assert_eq!(published, report);
    assert_eq!(
        fs::read_dir(output.path())
            .expect("list census output directory")
            .count(),
        1,
        "concurrent publication must not leave staging files"
    );
}
