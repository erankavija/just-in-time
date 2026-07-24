//! Property-based coverage for [`plan_hash`] determinism under action reordering.
//!
//! `RepositoryDelta::new` sorts its input actions by canonical path and rejects
//! duplicate targets, so a permutation of the same action set always normalizes
//! to one identical delta. These properties pin that boundary: [`plan_hash`] must
//! be invariant under permutation of a delta's action-set input order, and it
//! must not collapse genuinely distinct deltas to the same hash while doing so.

use super::*;
use crate::repository_state::RepositoryRootEvidence;
use proptest::prelude::*;

fn layout() -> RepositoryLayout {
    RepositoryLayout::new(
        RepositoryRootEvidence::new("/repo", "wt", true),
        RepositoryRootEvidence::new("/repo/.jit", "data", true),
    )
    .unwrap()
}

/// A minimal closed image: an empty capture over a zero budget. `plan_hash`
/// folds the image, seed, and intent in alongside the delta, but this property
/// only varies the delta, so the surrounding plan context stays fixed.
fn empty_image(layout: RepositoryLayout) -> RepositoryImage {
    let spec = CaptureSpec::phase_one(
        Vec::<VirtualPath>::new(),
        CaptureBudget {
            max_paths: 0,
            max_listings: 0,
            max_bytes: 0,
            max_depth: 0,
        },
    )
    .unwrap();
    RepositoryImage::close(
        layout,
        spec,
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap()
}

fn fixed_seed() -> RepositorySeed {
    RepositorySeed::new(
        RepositorySeedKind::Command {
            name: "plan-hash-property".into(),
        },
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .unwrap()
}

/// The delta action kinds this generator covers, one per `RepositoryAction`
/// variant, split so `WriteFile` exercises both preimages `validate_action`
/// accepts for it (`Absent` and `File`).
#[derive(Debug, Clone, Copy)]
enum ActionKind {
    CreateDirectory,
    WriteFileAbsent,
    WriteFileOverwrite,
    SetMode,
    DeleteFile,
}

fn action_kind_strategy() -> impl Strategy<Value = ActionKind> {
    prop_oneof![
        Just(ActionKind::CreateDirectory),
        Just(ActionKind::WriteFileAbsent),
        Just(ActionKind::WriteFileOverwrite),
        Just(ActionKind::SetMode),
        Just(ActionKind::DeleteFile),
    ]
}

/// A path is either root class, at a small fixed set of synthetic names. The
/// small domain (24 names x 2 roots) makes independently generated action
/// sets collide on paths often enough to exercise both branches of the
/// discriminating property below, while every name is always canonical.
fn virtual_path_strategy() -> impl Strategy<Value = VirtualPath> {
    (
        prop_oneof![
            Just(RepositoryRootClass::Worktree),
            Just(RepositoryRootClass::Data),
        ],
        0u32..24,
    )
        .prop_map(|(root, index)| {
            let name = format!("path-{index}");
            match root {
                RepositoryRootClass::Worktree => VirtualPath::worktree(name),
                RepositoryRootClass::Data => VirtualPath::data(name),
            }
            .expect("synthetic single-segment ascii names are always canonical")
        })
}

/// A content identity derived solely from `path`'s `Debug` form. Distinct
/// `VirtualPath` values always print distinct `Debug` output (root class and
/// relative text are both included verbatim), so this identity's `object`
/// field alone is already unique per unique path — clearing
/// `RepositoryDelta::reject_physical_aliases` without relying on hashing luck.
fn identity_for(path: &VirtualPath) -> EntryIdentity {
    let discriminator = format!("{path:?}");
    EntryIdentity::for_bytes(format!("obj:{discriminator}"), discriminator.as_bytes())
        .expect("path-derived identity text is non-empty and control-character-free")
}

/// Build one action whose preimage kind matches what `validate_action`
/// requires for `kind`, per REQ-01. `File`-preimage variants use
/// [`identity_for`] so the action set's identities stay unique by
/// construction, per REQ-01's alias-free requirement.
fn build_action(path: VirtualPath, kind: ActionKind) -> RepositoryAction {
    let owner = "plan-hash-property";
    match kind {
        ActionKind::CreateDirectory => RepositoryAction::CreateDirectory {
            path,
            owner: owner.into(),
            expected: ExpectedPreimage::Absent,
        },
        ActionKind::WriteFileAbsent => RepositoryAction::WriteFile {
            path,
            owner: owner.into(),
            expected: ExpectedPreimage::Absent,
            bytes: b"created".to_vec(),
            mode: FileMode::Regular,
        },
        ActionKind::WriteFileOverwrite => {
            let identity = identity_for(&path);
            RepositoryAction::WriteFile {
                path,
                owner: owner.into(),
                expected: ExpectedPreimage::File {
                    identity,
                    mode: FileMode::Regular,
                },
                bytes: b"overwritten".to_vec(),
                mode: FileMode::Executable,
            }
        }
        ActionKind::SetMode => {
            let identity = identity_for(&path);
            RepositoryAction::SetMode {
                path,
                owner: owner.into(),
                expected: ExpectedPreimage::File {
                    identity,
                    mode: FileMode::Regular,
                },
                mode: FileMode::Executable,
            }
        }
        ActionKind::DeleteFile => {
            let identity = identity_for(&path);
            RepositoryAction::DeleteFile {
                path,
                owner: owner.into(),
                expected: ExpectedPreimage::File {
                    identity,
                    mode: FileMode::Regular,
                },
            }
        }
    }
}

/// One contract-valid, distinct-path action set (via the `BTreeMap` key),
/// paired with one shuffle key per action. Sorting the actions by their
/// paired key yields a uniformly random permutation of the same multiset.
fn action_set_with_shuffle_keys() -> impl Strategy<Value = (Vec<RepositoryAction>, Vec<u64>)> {
    prop::collection::btree_map(virtual_path_strategy(), action_kind_strategy(), 1..12)
        .prop_flat_map(|actions_by_path| {
            let actions: Vec<RepositoryAction> = actions_by_path
                .into_iter()
                .map(|(path, kind)| build_action(path, kind))
                .collect();
            let len = actions.len();
            prop::collection::vec(any::<u64>(), len).prop_map(move |keys| (actions.clone(), keys))
        })
}

proptest! {
    /// REQ-02: `plan_hash` is invariant under permutation of a delta's
    /// action-set input order, when both orderings are built through
    /// `RepositoryDelta::new`.
    #[test]
    fn prop_plan_hash_invariant_under_action_reorder(
        (actions, keys) in action_set_with_shuffle_keys(),
    ) {
        let repo_layout = layout();
        let original = RepositoryDelta::new(&repo_layout, actions.clone())
            .expect("generator produces contract-valid, alias-free actions");

        let mut keyed: Vec<(RepositoryAction, u64)> = actions.into_iter().zip(keys).collect();
        keyed.sort_by_key(|(_, key)| *key);
        let reordered_actions: Vec<RepositoryAction> =
            keyed.into_iter().map(|(action, _)| action).collect();
        let reordered = RepositoryDelta::new(&repo_layout, reordered_actions)
            .expect("reordering the same action set stays contract-valid");

        // Same multiset, permuted input order: RepositoryDelta::new's internal
        // sort must normalize both to one identical delta.
        prop_assert_eq!(&original, &reordered);

        let repo_image = empty_image(repo_layout);
        let repo_seed = fixed_seed();
        let intent = MaterializationIntent::SemanticMutation;

        let original_hash = plan_hash(&repo_image, &repo_seed, &intent, &original).unwrap();
        let reordered_hash = plan_hash(&repo_image, &repo_seed, &intent, &reordered).unwrap();

        prop_assert_eq!(original_hash, reordered_hash);
    }
}

proptest! {
    /// REQ-03: `plan_hash` equality exactly tracks normalized-`RepositoryDelta`
    /// equality (over a fixed image/seed/intent), in both directions. This
    /// guards the property above against a vacuous pass: a `plan_hash` that
    /// collapsed every delta to one constant hash would satisfy reorder
    /// invariance trivially, but would fail this property the first time two
    /// independently generated action sets turn out unequal.
    #[test]
    fn prop_plan_hash_equality_matches_normalized_delta_equality(
        left_by_path in prop::collection::btree_map(virtual_path_strategy(), action_kind_strategy(), 0..8),
        right_by_path in prop::collection::btree_map(virtual_path_strategy(), action_kind_strategy(), 0..8),
    ) {
        let repo_layout = layout();
        let left_actions: Vec<RepositoryAction> = left_by_path
            .into_iter()
            .map(|(path, kind)| build_action(path, kind))
            .collect();
        let right_actions: Vec<RepositoryAction> = right_by_path
            .into_iter()
            .map(|(path, kind)| build_action(path, kind))
            .collect();

        let left = RepositoryDelta::new(&repo_layout, left_actions)
            .expect("generator produces contract-valid, alias-free actions");
        let right = RepositoryDelta::new(&repo_layout, right_actions)
            .expect("generator produces contract-valid, alias-free actions");

        let repo_image = empty_image(repo_layout);
        let repo_seed = fixed_seed();
        let intent = MaterializationIntent::SemanticMutation;

        let left_hash = plan_hash(&repo_image, &repo_seed, &intent, &left).unwrap();
        let right_hash = plan_hash(&repo_image, &repo_seed, &intent, &right).unwrap();

        prop_assert_eq!(
            left_hash == right_hash,
            left == right,
            "plan_hash equality must exactly track normalized RepositoryDelta equality"
        );
    }
}
