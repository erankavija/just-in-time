//! Selection oracle from `selection-membership` in the 9876a23a plan.

use super::report::{
    BuiltInPass, FindingOrigin, MissingHistoryError, ReportedFinding, RuleSelectionFault,
    Selection, StructuralFailure,
};
use super::{report, CapturedValidation, HistoryVerdicts, ReportInputs, ValidationReport};
use crate::declarations::rules::{RuleScope, RuleSet, Severity};
use crate::domain::identity::{IssueId, IssueIdPrefix};
use crate::domain::{Issue, State};
use crate::repository_state::{DirectoryListing, RepositoryImage, VirtualPath};
use crate::test_harness::{arbitrary_dag, image::ImageBuilder};
use proptest::prelude::*;
use std::collections::HashSet;

const TEMPLATES: &str = r#"
[[template]]
name = "bracket"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "planning"
doc = "plans/{container.id}.md"
[[template.nodes]]
role = "breakdown"
type = "breakdown"
labels = ["brackets:{container.short_id}"]
depends_on = ["planning"]
"#;

const MENU: [(&str, &str); 4] = [
    ("local", r#"json-schema = "schemas/local.json""#),
    (
        "coverage",
        r#"label-coverage = { marker = "[hard]", id-pattern = "REQ-[0-9]+", child-link = "dependencies", container-from-label = "brackets" }"#,
    ),
    (
        "unique",
        r#"label-uniqueness = { namespace = "req", scope = "all" }"#,
    ),
    (
        "shape",
        r#"dependency-shape = { target = { type = "planning" }, mode = "must" }"#,
    ),
];

/// The absent plan a done planning node links, failing the rule pass.
const MISSING_PLAN: &str = "plans/missing.md";

const LOCAL_SCHEMA: &str = r#"{"properties":{"title":{"minLength":4}}}"#;

#[derive(Debug)]
struct Case {
    image: RepositoryImage,
    issues: Vec<Issue>,
    rules: Option<RuleSet>,
    broken_records: bool,
    rule_pass_fails: bool,
}

fn arbitrary_case() -> impl Strategy<Value = Case> {
    arbitrary_dag()
        .prop_filter("at least two issues", |nodes| nodes.len() >= 2)
        .prop_flat_map(|nodes| {
            let n = nodes.len();
            (
                Just(nodes),
                prop::collection::vec((0u8..6, any::<bool>(), 0u8..3), n),
                prop::array::uniform4((0u8..3, 0u8..4)),
                0u8..5,
                any::<bool>(),
                0u8..8,
                0u8..4,
            )
        })
        .prop_map(|(nodes, attributes, rule_options, rules_mode, placeholder, records_mode, plan_mode)| {
            let ids: Vec<IssueId> = (0..nodes.len())
                .map(|i| {
                    let head = if i < 2 { 0xa0000000 } else { 0xb0000000 + i };
                    format!("{head:08x}-0000-0000-0000-{i:012x}")
                        .parse()
                        .unwrap()
                })
                .collect();
            let mut issues: Vec<Issue> = nodes
                .iter()
                .enumerate()
                .map(|(i, (kind, dependencies))| {
                    let (pointer, done, link) = attributes[i];
                    let kind = if i == nodes.len() - 1 {
                        "epic"
                    } else if i == 0 {
                        "breakdown"
                    } else {
                        match kind {
                            Some(1) => "epic",
                            Some(2) => "planning",
                            Some(3) => "breakdown",
                            _ => "task",
                        }
                    };
                    let mut issue = Issue::draft(
                        ids[i].clone(),
                        if done { "long title" } else { "x" }.into(),
                        "## Success Criteria\n\n- [hard] REQ-01: covered\n".into(),
                    );
                    issue.state = if done { State::Done } else { State::Backlog };
                    issue.dependencies = dependencies.iter().map(|&j| ids[j].clone()).collect();
                    issue.labels = vec![format!("type:{kind}"), format!("req:{}", i % 2)];
                    match pointer {
                        0 => issue.labels.push(format!("brackets:{}", ids[nodes.len() - 1])),
                        1 => issue.labels.push(format!("brackets:{}", ids[0])),
                        2 => issue.labels.push("brackets:a0000000".into()),
                        3 => issue.labels.push("brackets:ffffffff".into()),
                        4 => issue.labels.extend([
                            format!("brackets:{}", ids[0]),
                            format!("brackets:{}", ids[1]),
                        ]),
                        _ => {}
                    }
                    issue.labels.extend([
                        format!("satisfies:@/issue/{}/requirement/REQ-01", ids[0]),
                        "enforces:@/invariant/rule-local".into(),
                        "enforces:@/invariant/missing".into(),
                        "satisfies:REQ-01".into(),
                    ]);
                    issue.labels.push(match link {
                        0 => format!("satisfies:{}/REQ-01", ids[1]),
                        1 => format!("satisfies:@/issue/{}/requirement/MISSING", ids[1]),
                        _ => "enforces:rule-local".into(),
                    });
                    issue
                })
                .collect();
            let plan_missing = plan_mode == 0;
            if plan_missing {
                issues = missing_plan_bracket();
            }
            let rules_text: String = MENU
                .iter()
                .zip(rule_options)
                .map(|((name, assertion), (severity, selector))| {
                    let severity = ["off", "warn", "error"][severity as usize];
                    let selector = ["", "type = \"task\"", "type = \"breakdown\"", "type = \"epic\""]
                        [selector as usize];
                    format!(
                        "[[rules]]\nname = {name:?}\nseverity = {severity:?}\nwhen = {{ {selector} }}\nassert = {{ {assertion} }}\n"
                    )
                })
                .collect();
            let config = format!(
                "[type_hierarchy.types]\nepic = 1\nplanning = 2\nbreakdown = 3\ntask = 4\n{}",
                crate::test_utils::shipped_item_kind_tables(&["requirement", "invariant"])
            );
            let invariants: String = MENU
                .iter()
                .map(|(name, _)| {
                    format!(
                        "[[invariants]]\nid = \"rule-{name}\"\nstatement = \"s\"\nkind = \"enforced\"\nenforced-by = \"@/rule/{name}\"\n"
                    )
                })
                .chain(std::iter::once(
                    "[[invariants]]\nid = \"gate-review\"\nstatement = \"s\"\nkind = \"enforced\"\nenforced-by = \"@/gate/review\"\n".into(),
                ))
                .collect();
            let gates = if placeholder {
                "[[gates]]\nversion = 1\nkey = \"review\"\ntitle = \"Review\"\ndescription = \"Review\"\nstage = \"postcheck\"\nmode = \"manual\"\nchecker = { type = \"review_placeholder\" }\n"
            } else {
                ""
            };
            let broken_records = records_mode == 0;
            let builder = ImageBuilder::new()
                .file(VirtualPath::CONFIG, config)
                .file(VirtualPath::TEMPLATES, TEMPLATES)
                .file(VirtualPath::INVARIANTS, invariants)
                .file(VirtualPath::GATES, gates)
                .file(VirtualPath::schema("local.json").unwrap(), LOCAL_SCHEMA)
                .file(VirtualPath::RULES, if rules_mode != 0 { &rules_text } else { "not toml = [" })
                .file(VirtualPath::INDEX, crate::repository_state::fresh_index_bytes().unwrap())
                .listing(
                    VirtualPath::ISSUES,
                    DirectoryListing::new(issues.iter().map(|issue| format!("{}.json", issue.id)).collect::<Vec<_>>()).unwrap(),
                )
                .files(&[(MISSING_PLAN, None)]);
            let image = issues.iter().enumerate().fold(builder, |builder, (i, issue)| {
                builder.file(
                    VirtualPath::issue_record(&issue.id),
                    if broken_records && i == 0 { b"not json".to_vec() } else { crate::repository_state::serialize_issue(issue).unwrap() },
                )
            }).build();
            let declarations = crate::repository_state::validation_declarations_with_config(&image, crate::repository_state::assemble_config(&image)).unwrap();
            let rules = declarations.rules_loaded().then(|| declarations.rules().clone());
            let rule_pass_fails = plan_missing && rules.is_some() && !broken_records;
            Case { image, issues, rules, broken_records, rule_pass_fails }
        })
}

/// An integrity-clean bracket whose done planning node links [`MISSING_PLAN`],
/// so the rule pass is the first structural failure.
fn missing_plan_bracket() -> Vec<Issue> {
    let node = |head: u32, index: usize, kind: &str, state: State, dependencies: &[&Issue]| {
        let id = format!("{head:08x}-0000-0000-0000-{index:012x}")
            .parse()
            .unwrap();
        let mut issue = Issue::draft(id, kind.into(), String::new());
        issue.labels = vec![format!("type:{kind}")];
        issue.state = state;
        issue.dependencies = dependencies.iter().map(|issue| issue.id.clone()).collect();
        issue
    };
    let mut planning = node(0xa0000000, 0, "planning", State::Done, &[]);
    planning.documents.push(crate::domain::DocumentReference {
        label: Some("plan".into()),
        ..crate::test_utils::document_reference(MISSING_PLAN)
    });
    let mut breakdown = node(0xa0000000, 1, "breakdown", State::Ready, &[&planning]);
    let epic = node(0xb0000002, 2, "epic", State::Backlog, &[&breakdown]);
    breakdown
        .labels
        .push(format!("brackets:{}", epic.short_id()));
    vec![planning, breakdown, epic]
}

#[derive(Debug, PartialEq)]
enum Fault {
    History,
    Records,
    RulePass,
    Missing,
    Ambiguous,
    Rule,
}

fn resolve<'a>(case: &'a Case, prefix: &IssueIdPrefix) -> Result<&'a Issue, Fault> {
    let matches: Vec<_> = case
        .issues
        .iter()
        .filter(|issue| issue.id.as_str().starts_with(prefix.as_str()))
        .collect();
    match matches.as_slice() {
        [] => Err(Fault::Missing),
        [issue] => Ok(issue),
        _ => Err(Fault::Ambiguous),
    }
}

// Each arm encodes a clause of selection-membership; no production membership call.
fn oracle(
    case: &Case,
    report: &ValidationReport,
    selection: &Selection,
) -> Result<Vec<ReportedFinding>, Fault> {
    if *selection == Selection::Whole {
        return report
            .has_history
            .then(|| report.findings.iter().map(|f| f.reported.clone()).collect())
            .ok_or(Fault::History);
    }
    if case.broken_records {
        return Err(Fault::Records);
    }
    if case.rule_pass_fails {
        return Err(Fault::RulePass);
    }
    let prefix = match selection {
        Selection::Issue(prefix) | Selection::Scope(prefix) => prefix,
        Selection::Rule { issue, .. } => issue,
        Selection::Whole => unreachable!(),
    };
    let issue = resolve(case, prefix)?;
    if let (Selection::Rule { rule, .. }, Some(rules)) = (selection, &case.rules) {
        let selectable = rules.rules.iter().any(|r| {
            r.name == *rule
                && r.scope == RuleScope::Graph
                && r.severity != Severity::Off
                && r.when.matches(issue)
        });
        if !selectable {
            return Err(Fault::Rule);
        }
    }
    let indexed = crate::domain::queries::IndexedIssues::new(case.issues.clone());
    let members = crate::domain::queries::bracket_scope_ids(
        &issue.id,
        |id| indexed.get(id),
        Some("breakdown"),
    );
    let applies = |name: &str, ids: &HashSet<IssueId>| {
        case.rules.as_ref().is_some_and(|rules| {
            rules.rules.iter().any(|r| {
                r.name == name
                    && r.scope == RuleScope::Graph
                    && case
                        .issues
                        .iter()
                        .any(|i| ids.contains(&i.id) && r.when.matches(i))
            })
        })
    };
    Ok(report
        .findings
        .iter()
        .filter_map(|f| {
            let attributed = f.reported.issue_id.as_ref();
            let subject = f.subject.as_ref();
            let keep = match selection {
                Selection::Issue(_) => match f.origin {
                    FindingOrigin::BuiltIn(BuiltInPass::RulesFile) => true,
                    FindingOrigin::BuiltIn(
                        BuiltInPass::EnforcementDrift | BuiltInPass::ReviewPlaceholder,
                    ) => false,
                    FindingOrigin::Graph { .. } if f.is_config_error => {
                        applies(&f.reported.rule, &HashSet::from([issue.id.clone()]))
                            && (subject.is_none() || subject == Some(&issue.id))
                    }
                    _ => attributed == Some(&issue.id),
                },
                Selection::Scope(_) => match f.origin {
                    FindingOrigin::BuiltIn(
                        BuiltInPass::RulesFile | BuiltInPass::EnforcementDrift,
                    ) => true,
                    FindingOrigin::BuiltIn(BuiltInPass::ReviewPlaceholder) => false,
                    FindingOrigin::Graph { .. } if f.is_config_error => match subject {
                        Some(id) => members.contains(id),
                        None => applies(&f.reported.rule, &members),
                    },
                    FindingOrigin::Graph {
                        repo_wide_at_transition: true,
                    } => false,
                    _ => attributed.is_some_and(|id| members.contains(id)),
                },
                Selection::Rule { rule, .. } => match f.origin {
                    FindingOrigin::BuiltIn(BuiltInPass::RulesFile) => true,
                    FindingOrigin::Graph { .. } => {
                        f.reported.rule == *rule
                            && (subject == Some(&issue.id)
                                || subject.is_none() && f.is_config_error)
                    }
                    _ => false,
                },
                Selection::Whole => unreachable!(),
            };
            keep.then(|| ReportedFinding {
                issue_id: if matches!(selection, Selection::Issue(_)) {
                    Some(issue.id.clone())
                } else {
                    attributed.cloned()
                },
                ..f.reported.clone()
            })
        })
        .collect())
}

fn names_missing_plan(error: &anyhow::Error) -> bool {
    format!("{error:#}").contains(MISSING_PLAN)
}

fn check_selection(case: &Case, report: &ValidationReport, selection: &Selection) {
    let expected = oracle(case, report, selection);
    let actual = report.select(selection);
    match expected {
        Ok(findings) if *selection == Selection::Whole && report.structural_failure.is_some() => {
            let error = actual.unwrap_err();
            let structural = error.downcast_ref::<StructuralFailure>().unwrap();
            assert_eq!(structural.findings.findings, findings);
            assert_eq!(
                format!("{:#}", structural.failure),
                format!("{:#}", report.structural_failure.as_ref().unwrap())
            );
        }
        Ok(findings) => assert_eq!(actual.unwrap().findings, findings, "{selection:?}"),
        Err(fault) => {
            let error = actual.unwrap_err();
            match fault {
                Fault::History => assert!(error.is::<MissingHistoryError>()),
                Fault::Records => assert_eq!(
                    format!("{error:#}"),
                    format!("{:#}", report.structural_failure.as_ref().unwrap())
                ),
                Fault::RulePass => assert!(names_missing_plan(&error), "{error:#}"),
                Fault::Missing => assert!(error.is::<crate::storage::IssueNotFoundError>()),
                Fault::Ambiguous => assert!(error.is::<crate::storage::AmbiguousIdError>()),
                Fault::Rule => assert!(error.is::<RuleSelectionFault>()),
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn test_select_generated_images_obey_selection_membership(case in arbitrary_case()) {
        let captured = CapturedValidation::from_image(&case.image);
        for history_mode in 0..4 {
            let report = report(&captured, &ReportInputs {
                now: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
                history: (history_mode != 0).then(|| HistoryVerdicts {
                    record_fidelity: if history_mode == 2 { Err("fidelity failure".into()) } else { Ok(()) },
                    document_references: if history_mode == 3 { Err("document failure".into()) } else { Ok(()) },
                }),
            });
            let selectable = !case.broken_records && !case.rule_pass_fails;
            assert_eq!(report.selectable.is_ok(), selectable, "{report:?}");
            check_selection(&case, &report, &Selection::Whole);
            // Fidelity and document-reference verdicts precede the rule pass.
            if case.rule_pass_fails && history_mode == 1 {
                let error = report.select(&Selection::Whole).unwrap_err();
                let structural = error.downcast_ref::<StructuralFailure>().unwrap();
                assert!(names_missing_plan(&structural.failure), "{error:#}");
            }
            for prefix in ["a0000000", "ffffffff"] {
                let prefix: IssueIdPrefix = prefix.parse().unwrap();
                for selection in [Selection::Issue(prefix.clone()), Selection::Scope(prefix.clone()), Selection::Rule { issue: prefix.clone(), rule: "coverage".into() }] {
                    check_selection(&case, &report, &selection);
                }
                assert!(report.explain(&prefix).is_err());
            }
            let mut source_union: HashSet<usize> = report.findings.iter().enumerate()
                .filter(|(_, f)| f.reported.issue_id.is_none() || matches!(f.origin, FindingOrigin::BuiltIn(_)))
                .map(|(index, _)| index).collect();
            for issue in &case.issues {
                let prefix = IssueIdPrefix::from(&issue.id);
                let selection = Selection::Issue(prefix.clone());
                check_selection(&case, &report, &selection);
                if issue.labels.iter().any(|label| label == "type:epic") {
                    check_selection(&case, &report, &Selection::Scope(prefix.clone()));
                }
                let graph_rules = case.rules.iter().flat_map(|rules| &rules.rules)
                    .filter(|rule| rule.scope == RuleScope::Graph).map(|rule| rule.name.as_str());
                for rule in graph_rules.chain(["coverage", "unique", "shape", "local", "missing"]) {
                    check_selection(&case, &report, &Selection::Rule { issue: prefix.clone(), rule: rule.into() });
                }
                if selectable {
                    for finding in report.select(&selection).unwrap().findings {
                        let sources: Vec<_> = report.findings.iter().enumerate()
                            .filter(|(_, source)| ReportedFinding {
                                issue_id: Some(issue.id.clone()),
                                ..source.reported.clone()
                            } == finding)
                            .map(|(index, _)| index).collect();
                        assert!(!sources.is_empty(), "selected finding has no whole-report source: {finding:?}");
                        source_union.extend(sources);
                    }
                }
                let explained = report.explain(&prefix);
                if case.rule_pass_fails {
                    assert!(names_missing_plan(&explained.unwrap_err()));
                } else if case.broken_records || case.rules.is_none() {
                    assert!(explained.is_err());
                } else {
                    let selected = report.select(&selection).unwrap();
                    let explained = explained.unwrap();
                    let rules = &case.rules.as_ref().unwrap().rules;
                    assert_eq!(explained.outcomes.len(), rules.len());
                    for (outcome, rule) in explained.outcomes.iter().zip(rules) {
                        assert_eq!(outcome.rule, rule.name);
                        assert_eq!(outcome.matched, rule.when.matches(issue));
                        if outcome.matched {
                            assert_eq!(!outcome.passed, selected.findings.iter().any(|f| f.rule == rule.name));
                        }
                    }
                }
            }
            if selectable {
                assert_eq!(source_union, (0..report.findings.len()).collect(), "issue source union omitted a whole-report finding");
            }
        }
    }
}
