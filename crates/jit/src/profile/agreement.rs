//! Whether the profiles a repository records still agree with the packages they
//! came from and with the repository content they own.
//!
//! A repository states which profiles it carries in its own applied-profile
//! records, and each record carries the package identity it was applied from
//! plus one ownership claim per contribution it published, fingerprinted at the
//! value it published. Those two facts are what this module compares: the
//! package the record names against the identity it records, and each claim's
//! recorded base against the value the repository holds now.
//!
//! The claim comparison here is pure — it reads only a captured image — so the
//! profile-scoped check and repository-wide validation report the same
//! divergences without either resolving a package to ask the question.

use crate::domain::ProfileOrigin;
use crate::profile::ProfileCollection;
use crate::repository_state::{
    claimed_target_state, remedy_suggestions, AppliedProfileRecord, ClaimedTargetState,
    ProfileRemedy, ProfileResolution, RepositoryImage, RepositoryStateError,
};
use schemars::{gen::SchemaGenerator, schema::Schema, JsonSchema};
use serde::Serialize;

/// One way a recorded profile no longer agrees with its package or with the
/// repository content it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileDivergence {
    /// The location this profile's record names holds no readable package.
    ///
    /// The record's own origin states which location that is, so the reason
    /// carries what the read failed on rather than restating the path.
    UnreadablePackage {
        /// Why no package could be read at the recorded location.
        reason: String,
    },
    /// The package at the recorded location no longer accepts the values this
    /// profile resolved, so its identity cannot be compared at all.
    UnresolvedValues {
        /// Why the recorded values did not resolve.
        reason: String,
    },
    /// The recorded location holds a package other than the one this profile
    /// records.
    ChangedPackageIdentity {
        /// Package version the record carries.
        recorded_version: String,
        /// Package identity digest the record carries.
        recorded_package_hash: String,
        /// Package version the recorded location holds now.
        current_version: String,
        /// Package identity digest the recorded location holds now.
        current_package_hash: String,
    },
    /// A target this profile owns holds a value other than the one it published.
    ChangedTarget {
        /// Repository-relative name of the owned target.
        target: String,
    },
    /// A target this profile claims is no longer in the repository.
    AbsentTarget {
        /// Repository-relative name of the claimed target.
        target: String,
    },
    /// A target this profile claims whose package can no longer be read, so
    /// nothing remains to compare it against or restore it from.
    UnownedTarget {
        /// Repository-relative name of the claimed target.
        target: String,
    },
}

impl ProfileDivergence {
    /// The resolutions that apply to this divergence.
    pub fn remedy(&self) -> ProfileRemedy {
        match self {
            Self::UnreadablePackage { .. }
            | Self::UnresolvedValues { .. }
            | Self::UnownedTarget { .. } => {
                ProfileRemedy::new([ProfileResolution::RestoreRecordedPackage])
            }
            Self::ChangedPackageIdentity { .. }
            | Self::ChangedTarget { .. }
            | Self::AbsentTarget { .. } => ProfileRemedy::new([
                ProfileResolution::RestoreRecordedContent,
                ProfileResolution::CaptureRepositoryContent,
            ]),
        }
    }

    /// A standalone human sentence naming what diverged and how.
    ///
    /// The message carries every fact its structured form carries, so a reader
    /// of CLI output and a reader of a validation finding act on the same
    /// evidence without consulting the JSON envelope.
    pub fn message(&self) -> String {
        let finding = match self {
            Self::UnreadablePackage { reason } => reason.clone(),
            Self::UnresolvedValues { reason } => format!(
                "the values this profile recorded no longer resolve against the package at its \
                 recorded location: {reason}"
            ),
            Self::ChangedPackageIdentity {
                recorded_version,
                recorded_package_hash,
                current_version,
                current_package_hash,
            } => format!(
                "the recorded location holds version {current_version} ({current_package_hash}), \
                 not the version {recorded_version} ({recorded_package_hash}) this profile records"
            ),
            Self::ChangedTarget { target } => {
                format!("'{target}' no longer holds the value this profile published")
            }
            Self::AbsentTarget { target } => {
                format!("'{target}' is claimed by this profile and is no longer in the repository")
            }
            Self::UnownedTarget { target } => format!(
                "'{target}' is claimed by this profile, whose package can no longer be read"
            ),
        };
        format!("{finding}: {}", self.remedy().message())
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProfileDivergenceWire<'a> {
    UnreadablePackage {
        reason: &'a str,
        remedy: ProfileRemedy,
    },
    UnresolvedValues {
        reason: &'a str,
        remedy: ProfileRemedy,
    },
    ChangedPackageIdentity {
        recorded_version: &'a str,
        recorded_package_hash: &'a str,
        current_version: &'a str,
        current_package_hash: &'a str,
        remedy: ProfileRemedy,
    },
    ChangedTarget {
        target: &'a str,
        remedy: ProfileRemedy,
    },
    AbsentTarget {
        target: &'a str,
        remedy: ProfileRemedy,
    },
    UnownedTarget {
        target: &'a str,
        remedy: ProfileRemedy,
    },
}

impl Serialize for ProfileDivergence {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let wire = match self {
            Self::UnreadablePackage { reason } => ProfileDivergenceWire::UnreadablePackage {
                reason,
                remedy: self.remedy(),
            },
            Self::UnresolvedValues { reason } => ProfileDivergenceWire::UnresolvedValues {
                reason,
                remedy: self.remedy(),
            },
            Self::ChangedPackageIdentity {
                recorded_version,
                recorded_package_hash,
                current_version,
                current_package_hash,
            } => ProfileDivergenceWire::ChangedPackageIdentity {
                recorded_version,
                recorded_package_hash,
                current_version,
                current_package_hash,
                remedy: self.remedy(),
            },
            Self::ChangedTarget { target } => ProfileDivergenceWire::ChangedTarget {
                target,
                remedy: self.remedy(),
            },
            Self::AbsentTarget { target } => ProfileDivergenceWire::AbsentTarget {
                target,
                remedy: self.remedy(),
            },
            Self::UnownedTarget { target } => ProfileDivergenceWire::UnownedTarget {
                target,
                remedy: self.remedy(),
            },
        };
        wire.serialize(serializer)
    }
}

#[allow(dead_code)]
#[derive(JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProfileDivergenceSchema {
    UnreadablePackage {
        reason: String,
        remedy: ProfileRemedy,
    },
    UnresolvedValues {
        reason: String,
        remedy: ProfileRemedy,
    },
    ChangedPackageIdentity {
        recorded_version: String,
        recorded_package_hash: String,
        current_version: String,
        current_package_hash: String,
        remedy: ProfileRemedy,
    },
    ChangedTarget {
        target: String,
        remedy: ProfileRemedy,
    },
    AbsentTarget {
        target: String,
        remedy: ProfileRemedy,
    },
    UnownedTarget {
        target: String,
        remedy: ProfileRemedy,
    },
}

impl JsonSchema for ProfileDivergence {
    fn schema_name() -> String {
        "ProfileDivergence".to_string()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        ProfileDivergenceSchema::json_schema(generator)
    }
}

/// One recorded profile's agreement with its package and with the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProfileAgreement {
    /// Stable profile identifier the record names.
    pub id: String,
    /// Semantic version the applied record names.
    pub version: String,
    /// Repository-relative path of the applied-profile record.
    pub record: String,
    /// Where the record says its package was read from.
    pub origin: ProfileOrigin,
    /// Number of divergences in [`Self::divergences`].
    pub count: usize,
    /// Every way this profile diverged, package facts before owned targets and
    /// owned targets in canonical claim order.
    pub divergences: Vec<ProfileDivergence>,
}

impl ProfileAgreement {
    /// Collect one profile's divergences into its answer.
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        record: impl Into<String>,
        origin: ProfileOrigin,
        divergences: Vec<ProfileDivergence>,
    ) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            record: record.into(),
            origin,
            count: divergences.len(),
            divergences,
        }
    }

    /// Whether this profile still agrees with its package and the repository.
    pub fn agrees(&self) -> bool {
        self.divergences.is_empty()
    }
}

/// Count-wrapped agreement report over every profile a repository records.
///
/// A repository that records no profile reports no entry, because the records
/// are the whole inventory: nothing about the running binary or the packages
/// present in the worktree adds to it.
pub type ProfileAgreementResult = ProfileCollection<ProfileAgreement>;

impl ProfileCollection<ProfileAgreement> {
    /// Whether every recorded profile agrees.
    pub fn agrees(&self) -> bool {
        self.profiles.iter().all(ProfileAgreement::agrees)
    }

    /// The profiles that diverged, in report order.
    pub fn diverged(&self) -> impl Iterator<Item = &ProfileAgreement> {
        self.profiles.iter().filter(|profile| !profile.agrees())
    }

    /// Distinct resolutions across every divergence this report carries,
    /// naming capture only where some divergence's remedy allows it.
    pub fn suggestions(&self) -> Vec<String> {
        remedy_suggestions(
            self.profiles
                .iter()
                .flat_map(|profile| &profile.divergences)
                .map(ProfileDivergence::remedy),
        )
    }
}

/// Compare every target one record claims against the value `image` holds for
/// it.
///
/// The comparison is per claim, against that claim's own recorded base, so the
/// answer names which contribution diverged rather than which file changed —
/// a semantic declaration edited in its registry is reported as precisely as an
/// asset edited in place.
///
/// A claim whose target lies outside the capture yields nothing: the image is
/// not evidence about a path it never read, and reporting an unread path as
/// absent would turn a narrow capture into a divergence. A claim whose value is
/// composed with content another owner manages
/// ([`ClaimedTargetState::Composed`]) likewise yields nothing about its content,
/// while still being reported when the target it names is gone.
///
/// # Errors
///
/// Returns [`RepositoryStateError`] when a claimed registry is not a regular
/// file or does not parse as the declaration a claim's identity addresses.
pub fn claimed_target_divergences(
    image: &RepositoryImage,
    record: &AppliedProfileRecord,
) -> Result<Vec<ProfileDivergence>, RepositoryStateError> {
    record
        .claims
        .iter()
        .map(|claim| {
            Ok(match claimed_target_state(image, claim)? {
                ClaimedTargetState::Uncaptured
                | ClaimedTargetState::Composed
                | ClaimedTargetState::Unchanged => None,
                ClaimedTargetState::Absent => Some(ProfileDivergence::AbsentTarget {
                    target: claim.identity.to_string(),
                }),
                ClaimedTargetState::Changed => Some(ProfileDivergence::ChangedTarget {
                    target: claim.identity.to_string(),
                }),
            })
        })
        .filter_map(Result::transpose)
        .collect()
}

/// Report every target one record claims as unowned.
///
/// A record whose package cannot be read still names the contributions that
/// package published. Nothing remains to compare them against or to restore
/// them from, so each is reported in its own right rather than left implicit in
/// the unreadable package.
pub fn unowned_target_divergences(record: &AppliedProfileRecord) -> Vec<ProfileDivergence> {
    record
        .claims
        .iter()
        .map(|claim| ProfileDivergence::UnownedTarget {
            target: claim.identity.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ProfileAgreement, ProfileDivergence};
    use crate::domain::ProfileOrigin;
    use crate::repository_state::ProfileResolution;

    #[test]
    fn test_profile_divergence_serializes_the_remedy_that_its_message_renders() {
        let divergence = ProfileDivergence::ChangedTarget {
            target: "docs/guide.md".to_string(),
        };

        let serialized = serde_json::to_value(&divergence).expect("a divergence serializes");

        assert_eq!(serialized["kind"], "changed_target");
        assert_eq!(
            serialized["remedy"]["resolutions"],
            serde_json::json!(["restore-recorded-content", "capture-repository-content"])
        );
        assert_eq!(
            divergence.remedy().resolutions(),
            &[
                ProfileResolution::RestoreRecordedContent,
                ProfileResolution::CaptureRepositoryContent,
            ]
        );
        assert!(divergence
            .message()
            .contains(&divergence.remedy().message()));
    }

    #[test]
    fn test_profile_divergence_does_not_offer_capture_when_the_recorded_package_is_unavailable() {
        let divergence = ProfileDivergence::UnreadablePackage {
            reason: "profile package is gone".to_string(),
        };

        assert!(!divergence.remedy().allows_capture_repository_content());
        assert!(!divergence.message().contains("jit profile capture"));
    }

    #[test]
    fn test_profile_divergence_schema_exposes_the_typed_remedy() {
        let schema = serde_json::to_value(schemars::schema_for!(ProfileDivergence))
            .expect("the divergence schema serializes");

        assert!(
            schema.to_string().contains("remedy"),
            "the divergence schema describes the remedy beside the finding: {schema}"
        );
    }

    #[test]
    fn test_profile_agreement_result_suggestions_dedupes_and_marks_capture_only_when_applicable() {
        let origin = ProfileOrigin::Directory(
            crate::repository_state::RootRelativePath::parse("packages/captured")
                .expect("a canonical package location"),
        );
        let agreement = ProfileAgreement::new(
            "captured",
            "1.0.0",
            ".jit/profiles/captured.json",
            origin,
            vec![
                ProfileDivergence::ChangedTarget {
                    target: "docs/guide.md".to_string(),
                },
                ProfileDivergence::UnreadablePackage {
                    reason: "profile package is gone".to_string(),
                },
            ],
        );
        let result = super::ProfileAgreementResult::new(vec![agreement]);

        let suggestions = result.suggestions();

        assert!(
            suggestions
                .iter()
                .any(|text| text.contains("jit profile capture")),
            "a changed-target remedy allows capture: {suggestions:?}"
        );
        assert_eq!(
            suggestions
                .iter()
                .filter(|text| text.contains("restore"))
                .count(),
            2,
            "restoring content and restoring the package are distinct resolutions, each named \
             once: {suggestions:?}"
        );
    }

    #[test]
    fn test_profile_agreement_result_suggestions_is_empty_when_every_profile_agrees() {
        let origin = ProfileOrigin::Directory(
            crate::repository_state::RootRelativePath::parse("packages/captured")
                .expect("a canonical package location"),
        );
        let agreement = ProfileAgreement::new(
            "captured",
            "1.0.0",
            ".jit/profiles/captured.json",
            origin,
            Vec::new(),
        );
        let result = super::ProfileAgreementResult::new(vec![agreement]);

        assert!(result.suggestions().is_empty());
    }
}
