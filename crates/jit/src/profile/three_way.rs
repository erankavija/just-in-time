//! Pure base/current/candidate decisions for profile-owned content.
//!
//! The lifecycle layer supplies typed target, owner, and value identities. This
//! module deliberately knows nothing about repository I/O, package discovery,
//! or publication so every caller uses the same safe-change table.

/// One observed target value, including a target that is absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreeWayValue<Value> {
    /// The target has no value.
    Absent,
    /// The target has one typed value.
    Present(Value),
}

/// All information needed to decide one owned target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreeWayInput<Target, Owner, Value> {
    /// Stable identity of the target being decided.
    pub target: Target,
    /// Package whose recorded claim is being replaced.
    pub owner: Owner,
    /// Value recorded when the package last published this claim.
    pub base: ThreeWayValue<Value>,
    /// Value currently observed in the repository.
    pub current: ThreeWayValue<Value>,
    /// Value resolved from the replacement package, if it still contributes it.
    pub candidate: ThreeWayValue<Value>,
    /// Other packages that still own this target after replacement.
    pub surviving_owners: usize,
    /// Whether adoption requires the value to survive after its last owner stops.
    pub retain_if_unowned: bool,
}

/// An actionable concurrent-edit conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreeWayConflict<Target, Owner, Value> {
    /// Stable identity of the target that could not be safely changed.
    pub target: Target,
    /// Package whose replacement encountered the conflict.
    pub owner: Owner,
    /// Last value published by `owner`.
    pub base: ThreeWayValue<Value>,
    /// Value observed in the repository while planning.
    pub current: ThreeWayValue<Value>,
    /// Value resolved from the replacement package.
    pub candidate: ThreeWayValue<Value>,
}

/// The one safe outcome for a base/current/candidate comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreeWayDecision<Target, Owner, Value> {
    /// The candidate safely replaces the recorded base.
    Update,
    /// The current value already equals the candidate.
    Unchanged,
    /// The package stopped contributing the target, but its value must survive.
    Retain,
    /// The package stopped contributing unchanged, solely-owned, disposable content.
    Remove,
    /// A concurrent edit differs from both the recorded base and candidate.
    Conflict(ThreeWayConflict<Target, Owner, Value>),
}

/// Decide one profile target from its recorded base, current repository value,
/// and resolved candidate.
///
/// A present candidate follows ordinary three-way merge semantics: unchanged
/// repository content updates, an already-equal candidate is a no-op, and a
/// divergent concurrent edit conflicts. A missing candidate is deliberately
/// stricter: deletion is permitted only for unchanged, solely-owned content
/// that was not retained during adoption. Every other missing-candidate case
/// preserves the current repository value.
pub fn decide_three_way<Target, Owner, Value>(
    input: ThreeWayInput<Target, Owner, Value>,
) -> ThreeWayDecision<Target, Owner, Value>
where
    Target: Clone,
    Owner: Clone,
    Value: Eq,
{
    match input.candidate {
        ThreeWayValue::Absent => {
            if input.current == input.base
                && input.surviving_owners == 0
                && !input.retain_if_unowned
            {
                ThreeWayDecision::Remove
            } else {
                ThreeWayDecision::Retain
            }
        }
        candidate => {
            if input.current == candidate {
                ThreeWayDecision::Unchanged
            } else if input.current == input.base {
                ThreeWayDecision::Update
            } else {
                ThreeWayDecision::Conflict(ThreeWayConflict {
                    target: input.target,
                    owner: input.owner,
                    base: input.base,
                    current: input.current,
                    candidate,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(value: &str) -> ThreeWayValue<&str> {
        ThreeWayValue::Present(value)
    }

    fn input<'a>(
        base: ThreeWayValue<&'a str>,
        current: ThreeWayValue<&'a str>,
        candidate: ThreeWayValue<&'a str>,
        surviving_owners: usize,
        retain_if_unowned: bool,
    ) -> ThreeWayInput<&'static str, &'static str, &'a str> {
        ThreeWayInput {
            target: "docs/profile.txt",
            owner: "workflow",
            base,
            current,
            candidate,
            surviving_owners,
            retain_if_unowned,
        }
    }

    #[test]
    fn test_decide_three_way_exhaustively_classifies_safe_change_cases() {
        let cases = [
            (
                "unchanged base updates to candidate",
                input(value("base"), value("base"), value("candidate"), 0, false),
                ThreeWayDecision::Update,
            ),
            (
                "current candidate is already applied",
                input(
                    value("base"),
                    value("candidate"),
                    value("candidate"),
                    0,
                    false,
                ),
                ThreeWayDecision::Unchanged,
            ),
            (
                "unchanged candidate is a no-op",
                input(value("base"), value("base"), value("base"), 0, false),
                ThreeWayDecision::Unchanged,
            ),
            (
                "absent base creates candidate",
                input(
                    ThreeWayValue::Absent,
                    ThreeWayValue::Absent,
                    value("candidate"),
                    0,
                    false,
                ),
                ThreeWayDecision::Update,
            ),
            (
                "divergent current conflicts",
                input(
                    value("base"),
                    value("current"),
                    value("candidate"),
                    0,
                    false,
                ),
                ThreeWayDecision::Conflict(ThreeWayConflict {
                    target: "docs/profile.txt",
                    owner: "workflow",
                    base: value("base"),
                    current: value("current"),
                    candidate: value("candidate"),
                }),
            ),
            (
                "unchanged solely owned content removes",
                input(
                    value("base"),
                    value("base"),
                    ThreeWayValue::Absent,
                    0,
                    false,
                ),
                ThreeWayDecision::Remove,
            ),
            (
                "shared content survives a departing owner",
                input(
                    value("base"),
                    value("base"),
                    ThreeWayValue::Absent,
                    1,
                    false,
                ),
                ThreeWayDecision::Retain,
            ),
            (
                "retained adoption survives its last owner",
                input(value("base"), value("base"), ThreeWayValue::Absent, 0, true),
                ThreeWayDecision::Retain,
            ),
            (
                "changed content survives a departing owner",
                input(
                    value("base"),
                    value("current"),
                    ThreeWayValue::Absent,
                    0,
                    false,
                ),
                ThreeWayDecision::Retain,
            ),
            (
                "already deleted content has no removal side effect",
                input(
                    value("base"),
                    ThreeWayValue::Absent,
                    ThreeWayValue::Absent,
                    0,
                    false,
                ),
                ThreeWayDecision::Retain,
            ),
        ];

        for (name, input, expected) in cases {
            assert_eq!(decide_three_way(input), expected, "{name}");
        }
    }
}
