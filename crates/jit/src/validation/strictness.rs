//! Global validation strictness: the enforcement modulator layered on top of
//! each rule's per-rule `enforce` flag and `severity`.
//!
//! A rule already carries a [`Severity`] and
//! an `enforce` flag; together they decide, per rule, whether a violation blocks
//! a write or transition (only an `enforce = true` / `error` finding blocks). The
//! repository-wide `[validation].strictness` key ([`Strictness`]) modulates that
//! block/allow decision GLOBALLY, WITHOUT mutating any rule's severity or
//! `enforce` flag:
//!
//! - [`Strictness::Strict`] — ANY violation (warning or error, enforced or not)
//!   blocks.
//! - [`Strictness::Loose`] — only an enforced error blocks, which is
//!   behaviorally identical to having no strictness modulation at all.
//! - [`Strictness::Permissive`] — NOTHING blocks; every violation is advisory
//!   (reported as a warning).
//!
//! The single decision is [`Strictness::blocks`]; every enforcement choke point
//! (the write path via [`LocalEvaluation`](crate::validation::local::LocalEvaluation)
//! and the transition path in the command layer) routes its block/allow decision
//! through it so the three levels cannot drift apart.

use crate::declarations::rules::Severity;
use crate::domain::vocabulary::wire_vocabulary;

wire_vocabulary! {
    /// Repository-wide enforcement modulator from `[validation].strictness`.
    ///
    /// See the module documentation for the full semantics. An absent key resolves
    /// to [`Strictness::DEFAULT`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub enum Strictness {
        /// Any violation (warning or error, enforced or not) blocks.
        Strict = "strict",
        /// Only an enforced error blocks (identical to no modulation).
        Loose = "loose",
        /// Nothing blocks; every violation is advisory.
        Permissive = "permissive",
    }
}

impl Default for Strictness {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Strictness {
    /// Level applied when `[validation]` omits `strictness`: the pre-strictness
    /// block/allow behavior.
    pub const DEFAULT: Self = Self::Loose;

    /// Whether a violation from a rule with the given per-rule `enforce` flag and
    /// `severity` blocks the write or transition under this strictness level.
    ///
    /// This is the ONE modulated decision shared by every enforcement choke
    /// point. It reads — never mutates — the rule's `enforce` and `severity`:
    ///
    /// - [`Strictness::Strict`] blocks on any reported violation (a
    ///   [`Severity::Off`] rule reports nothing and so never reaches here).
    /// - [`Strictness::Loose`] blocks only when the rule enforces AND its
    ///   severity is [`Severity::Error`], the exact pre-strictness rule.
    /// - [`Strictness::Permissive`] never blocks.
    pub fn blocks(self, enforce: bool, severity: Severity) -> bool {
        match self {
            // An `Off` rule is filtered out before evaluation, so any finding
            // that reaches here has another severity; guard on `!= Off` anyway
            // so the predicate is total and self-explanatory.
            Strictness::Strict => severity != Severity::Off,
            Strictness::Loose => enforce && severity == Severity::Error,
            Strictness::Permissive => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loose_matches_pre_strictness_rule() {
        // Loose is the baseline: block IFF the rule enforces AND is error-severity.
        let s = Strictness::Loose;
        assert!(s.blocks(true, Severity::Error), "enforced error blocks");
        assert!(
            !s.blocks(false, Severity::Error),
            "non-enforced error does not block"
        );
        assert!(
            !s.blocks(true, Severity::Warn),
            "enforced warning does not block"
        );
        assert!(
            !s.blocks(false, Severity::Warn),
            "plain warning does not block"
        );
    }

    #[test]
    fn test_strict_blocks_every_violation() {
        let s = Strictness::Strict;
        assert!(s.blocks(true, Severity::Error));
        assert!(
            s.blocks(false, Severity::Error),
            "non-enforced error blocks"
        );
        assert!(s.blocks(true, Severity::Warn));
        assert!(s.blocks(false, Severity::Warn), "plain warning blocks");
    }

    #[test]
    fn test_permissive_blocks_nothing() {
        let s = Strictness::Permissive;
        assert!(
            !s.blocks(true, Severity::Error),
            "even an enforced error is advisory"
        );
        assert!(!s.blocks(false, Severity::Warn));
        assert!(!s.blocks(true, Severity::Warn));
    }

    #[test]
    fn test_levels_are_distinct_on_the_same_violation() {
        // A single warning-only violation must produce three different outcomes,
        // so no level is silently a duplicate of another.
        let warn_only = |s: Strictness| s.blocks(false, Severity::Warn);
        assert!(!warn_only(Strictness::Loose));
        assert!(warn_only(Strictness::Strict));
        assert!(!warn_only(Strictness::Permissive));

        // And a single enforced-error violation separates loose from permissive.
        let enforced_error = |s: Strictness| s.blocks(true, Severity::Error);
        assert!(enforced_error(Strictness::Loose));
        assert!(!enforced_error(Strictness::Permissive));
    }

    #[test]
    fn test_default_impl_is_the_named_default() {
        assert_eq!(Strictness::default(), Strictness::DEFAULT);
    }
}
