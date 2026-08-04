//! The shape a repository-local check reports drift in.
//!
//! This repository carries several declarations twice: a packaged copy an
//! adopter receives, and a repository copy this checkout's own tooling reads.
//! A check that binds such a pair has the same thing to say when they come
//! apart — which two files carry the declaration, how each names the entry
//! inside it, what differs, and what brings the two back into agreement — so
//! that shape lives here once and each check supplies its own subject and its
//! own two values.
//!
//! Everything here is pure: the values arrive as arguments and nothing is read
//! from the filesystem. The module compiles only for the crate's own tests and
//! the dev-dependency-active builds those need, so an adopter build carries
//! none of it.

use serde_json::Value;

/// One carrier of a declaration: the file holding it, and how that file names
/// the entry inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftCarrier {
    /// Repository-relative path of the file.
    pub path: String,
    /// The entry inside it, spelled the way its file spells it.
    pub entry: String,
}

impl DriftCarrier {
    /// A carrier at `path` naming `entry`.
    pub fn new(path: impl Into<String>, entry: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            entry: entry.into(),
        }
    }
}

impl std::fmt::Display for DriftCarrier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} — {}", self.path, self.entry)
    }
}

/// One pair of carriers a check binds, with the repair that closes a gap
/// between them.
///
/// A subject is stated before the values are known, so the same subject reports
/// a value comparison ([`DriftSubject::compare`]) or a difference a check
/// derived some other way ([`DriftSubject::reporting`]).
#[derive(Debug, Clone)]
pub struct DriftSubject {
    /// The copy this repository's own tooling reads.
    pub repository: DriftCarrier,
    /// The copy an adopter receives.
    pub packaged: DriftCarrier,
    /// Root of the field paths the report names, spelled the way a carrier
    /// spells the declaration (`invariants[domain-agnostic]`, `template`).
    pub field_root: String,
    /// What a reader does to bring the two carriers back into agreement.
    pub remedy: String,
}

impl DriftSubject {
    /// The report this subject's two values produce, or `None` when every
    /// field agrees.
    ///
    /// Objects are compared over the union of their keys and arrays position by
    /// position, so a field one side omits and a position one side does not
    /// reach are both reported where they belong rather than collapsing the
    /// whole declaration into one difference.
    pub fn compare(self, repository: &Value, packaged: &Value) -> Option<DriftReport> {
        let differences = value_differences(&self.field_root, repository, packaged);
        self.reporting(differences)
    }

    /// The report `differences` produce, or `None` when there are none.
    pub fn reporting(self, differences: Vec<String>) -> Option<DriftReport> {
        (!differences.is_empty()).then_some(DriftReport {
            subject: self,
            differences,
        })
    }
}

/// Two carriers of one declaration that do not state the same thing.
///
/// The [`Display`](std::fmt::Display) rendering names both carriers and every
/// differing field with the value each side declares, so a reader repairs the
/// pair without first searching for the second copy.
#[derive(Debug, Clone)]
pub struct DriftReport {
    subject: DriftSubject,
    differences: Vec<String>,
}

impl DriftReport {
    /// The copy this repository's own tooling reads.
    pub fn repository(&self) -> &DriftCarrier {
        &self.subject.repository
    }

    /// The copy an adopter receives.
    pub fn packaged(&self) -> &DriftCarrier {
        &self.subject.packaged
    }

    /// Every field at which the two carriers disagree, each naming its path
    /// through the declaration and the value both sides declare.
    pub fn differences(&self) -> &[String] {
        &self.differences
    }
}

impl std::fmt::Display for DriftReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the repository's declaration and the packaged one disagree:\n  \
             repository: {}\n  packaged:   {}\n  {}\n{}",
            self.subject.repository,
            self.subject.packaged,
            self.differences.join("\n  "),
            self.subject.remedy,
        )
    }
}

/// Every field at which two compared values differ, each named by its path
/// through the declaration and shown from both sides.
fn value_differences(path: &str, repository: &Value, packaged: &Value) -> Vec<String> {
    match (repository, packaged) {
        (left, right) if left == right => Vec::new(),
        (Value::Object(left), Value::Object(right)) => left
            .keys()
            .chain(right.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .flat_map(|key| {
                member_differences(&format!("{path}.{key}"), left.get(key), right.get(key))
            })
            .collect(),
        (Value::Array(left), Value::Array(right)) => (0..left.len().max(right.len()))
            .flat_map(|index| {
                member_differences(
                    &format!("{path}[{index}]"),
                    left.get(index),
                    right.get(index),
                )
            })
            .collect(),
        (left, right) => vec![format!(
            "{path}: the repository declares {left}, the package declares {right}"
        )],
    }
}

/// The differences at one field path, where either side may be absent.
fn member_differences(
    path: &str,
    repository: Option<&Value>,
    packaged: Option<&Value>,
) -> Vec<String> {
    match (repository, packaged) {
        (Some(left), Some(right)) => value_differences(path, left, right),
        (Some(left), None) => vec![format!(
            "{path}: the repository declares {left}, the package declares nothing"
        )],
        (None, Some(right)) => vec![format!(
            "{path}: the repository declares nothing, the package declares {right}"
        )],
        (None, None) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The subject two agreeing values produce no report for.
    fn subject() -> DriftSubject {
        DriftSubject {
            repository: DriftCarrier::new(".jit/invariants.toml", "invariants entry 'a-property'"),
            packaged: DriftCarrier::new("profiles/p/manifest.toml", "contribution 'a-property'"),
            field_root: "invariants[a-property]".to_string(),
            remedy: "edit both carriers together".to_string(),
        }
    }

    #[test]
    fn test_compare_reports_nothing_when_the_two_carriers_agree() {
        assert!(subject()
            .compare(&json!({"statement": "a"}), &json!({"statement": "a"}))
            .is_none());
    }

    /// A report names both carriers, the differing field, and both values, so
    /// the second copy never has to be searched for.
    #[test]
    fn test_report_display_names_both_carriers_and_the_differing_text() {
        let report = subject()
            .compare(
                &json!({"statement": "the repository ruling"}),
                &json!({"statement": "the packaged restatement"}),
            )
            .expect("differing statements are drift");
        let rendered = report.to_string();

        for named in [
            report.repository().path.as_str(),
            report.repository().entry.as_str(),
            report.packaged().path.as_str(),
            report.packaged().entry.as_str(),
            "the repository ruling",
            "the packaged restatement",
        ] {
            assert!(rendered.contains(named), "{rendered}");
        }
        assert_eq!(report.differences().len(), 1);
    }

    /// A field only one side carries is reported at its own path, naming the
    /// side that lacks it, rather than collapsing the entry into one line.
    #[test]
    fn test_compare_reports_a_field_only_one_carrier_declares() {
        let report = subject()
            .compare(
                &json!({"statement": "a", "kind": "advisory"}),
                &json!({"statement": "a"}),
            )
            .expect("a field only the repository declares is drift");

        assert_eq!(report.differences().len(), 1);
        assert!(
            report.differences()[0].contains("kind")
                && report.differences()[0].contains("the package declares nothing"),
            "{:?}",
            report.differences()
        );
    }

    /// A difference a check derived some other way reports through the same
    /// shape, so every check names its carriers the same way.
    #[test]
    fn test_reporting_carries_a_difference_the_caller_derived() {
        let report = subject()
            .reporting(vec!["the repository's array does not carry 'a'".to_string()])
            .expect("a stated difference is drift");

        assert!(report.to_string().contains("does not carry 'a'"));
        assert!(subject().reporting(Vec::new()).is_none());
    }
}
