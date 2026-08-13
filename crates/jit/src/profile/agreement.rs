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
    claimed_target_state, AppliedProfileRecord, ClaimedTargetState, RepositoryImage,
    RepositoryStateError,
};
use schemars::JsonSchema;
use serde::Serialize;

/// One way a recorded profile no longer agrees with its package or with the
/// repository content it owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
    /// A standalone human sentence naming what diverged and how.
    ///
    /// The message carries every fact its structured form carries, so a reader
    /// of CLI output and a reader of a validation finding act on the same
    /// evidence without consulting the JSON envelope.
    pub fn message(&self) -> String {
        match self {
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
        }
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
