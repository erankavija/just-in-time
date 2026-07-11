//! Global validation strictness: the enforcement modulator layered on top of
//! each rule's per-rule `enforce` flag and `severity`.
//!
//! A rule already carries a [`Severity`](crate::validation::rules::Severity) and
//! an `enforce` flag; together they decide, per rule, whether a violation blocks
//! a write or transition (only an `enforce = true` / `error` finding blocks). The
//! repository-wide `[validation].strictness` key ([`Strictness`]) modulates that
//! block/allow decision GLOBALLY, WITHOUT mutating any rule's severity or
//! `enforce` flag:
//!
//! - [`Strictness::Strict`] — ANY violation (warning or error, enforced or not)
//!   blocks.
//! - [`Strictness::Loose`] — only an enforced error blocks. This is the default
//!   and is behaviorally identical to having no strictness modulation at all, so
//!   an existing repository keeps behaving exactly as before.
//! - [`Strictness::Permissive`] — NOTHING blocks; every violation is advisory
//!   (reported as a warning).
//!
//! The single decision is [`Strictness::blocks`]; every enforcement choke point
//! (the write path via [`LocalEvaluation`](crate::validation::local::LocalEvaluation)
//! and the transition path in the command layer) routes its block/allow decision
//! through it so the three levels cannot drift apart.

use std::str::FromStr;

use crate::errors::InvalidArgumentError;
use crate::validation::rules::Severity;

/// Repository-wide enforcement modulator from `[validation].strictness`.
///
/// See the module documentation for the full semantics. The default is
/// [`Strictness::Loose`], so an absent key preserves the pre-strictness
/// block/allow behavior.
///
/// # Examples
///
/// ```
/// use jit::validation::strictness::Strictness;
/// use jit::validation::rules::Severity;
///
/// // Loose (the default): only an enforced error blocks.
/// let loose = Strictness::default();
/// assert!(loose.blocks(true, Severity::Error));
/// assert!(!loose.blocks(false, Severity::Error));
/// assert!(!loose.blocks(true, Severity::Warn));
///
/// // Strict: any violation blocks, regardless of `enforce`.
/// assert!(Strictness::Strict.blocks(false, Severity::Warn));
///
/// // Permissive: nothing blocks.
/// assert!(!Strictness::Permissive.blocks(true, Severity::Error));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Strictness {
    /// Any violation (warning or error, enforced or not) blocks.
    Strict,
    /// Only an enforced error blocks (the default; identical to no modulation).
    #[default]
    Loose,
    /// Nothing blocks; every violation is advisory.
    Permissive,
}

impl Strictness {
    /// Whether a violation from a rule with the given per-rule `enforce` flag and
    /// `severity` blocks the write or transition under this strictness level.
    ///
    /// This is the ONE modulated decision shared by every enforcement choke
    /// point. It reads — never mutates — the rule's `enforce` and `severity`:
    ///
    /// - [`Strictness::Strict`] blocks on any reported violation (a `warn` or
    ///   `error` finding; an `off` rule reports nothing and so never reaches
    ///   here).
    /// - [`Strictness::Loose`] blocks only when the rule enforces AND its
    ///   severity is `error` — the exact pre-strictness rule.
    /// - [`Strictness::Permissive`] never blocks.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::strictness::Strictness;
    /// use jit::validation::rules::Severity;
    ///
    /// // The same warning-only violation is inert under loose, blocking under
    /// // strict, and advisory under permissive.
    /// assert!(!Strictness::Loose.blocks(false, Severity::Warn));
    /// assert!(Strictness::Strict.blocks(false, Severity::Warn));
    /// assert!(!Strictness::Permissive.blocks(false, Severity::Warn));
    /// ```
    pub fn blocks(self, enforce: bool, severity: Severity) -> bool {
        match self {
            // An `off` rule is filtered out before evaluation, so any finding
            // that reaches here is `warn` or `error`; guard on `!= Off` anyway so
            // the predicate is total and self-explanatory.
            Strictness::Strict => severity != Severity::Off,
            Strictness::Loose => enforce && severity == Severity::Error,
            Strictness::Permissive => false,
        }
    }

    /// Stable lowercase token for this level, matching the `config.toml` grammar.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::strictness::Strictness;
    ///
    /// assert_eq!(Strictness::Strict.token(), "strict");
    /// assert_eq!(Strictness::Loose.token(), "loose");
    /// assert_eq!(Strictness::Permissive.token(), "permissive");
    /// ```
    pub fn token(self) -> &'static str {
        match self {
            Strictness::Strict => "strict",
            Strictness::Loose => "loose",
            Strictness::Permissive => "permissive",
        }
    }

    /// Resolve a strictness level from the optional `[validation].strictness`
    /// config value, defaulting to [`Strictness::Loose`] when the key is absent.
    ///
    /// An invalid value is surfaced as an error rather than silently defaulting,
    /// so a misconfigured `config.toml` cannot quietly pick a strictness the
    /// author did not intend.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::strictness::Strictness;
    ///
    /// assert_eq!(Strictness::from_config_value(None).unwrap(), Strictness::Loose);
    /// assert_eq!(
    ///     Strictness::from_config_value(Some("strict")).unwrap(),
    ///     Strictness::Strict
    /// );
    /// assert!(Strictness::from_config_value(Some("banana")).is_err());
    /// ```
    pub fn from_config_value(value: Option<&str>) -> anyhow::Result<Self> {
        match value {
            None => Ok(Strictness::Loose),
            Some(raw) => Strictness::from_str(raw),
        }
    }
}

impl FromStr for Strictness {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        match s.to_lowercase().as_str() {
            "strict" => Ok(Strictness::Strict),
            "loose" => Ok(Strictness::Loose),
            "permissive" => Ok(Strictness::Permissive),
            _ => Err(InvalidArgumentError::new(format!(
                "invalid [validation].strictness in .jit/config.toml: '{s}' \
                 (expected strict, loose, or permissive)"
            ))
            .into()),
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
    fn test_default_is_loose() {
        assert_eq!(Strictness::default(), Strictness::Loose);
    }

    #[test]
    fn test_from_config_value_parses_each_level() {
        assert_eq!(
            Strictness::from_config_value(None).unwrap(),
            Strictness::Loose,
            "absent key defaults to loose"
        );
        assert_eq!(
            Strictness::from_config_value(Some("strict")).unwrap(),
            Strictness::Strict
        );
        assert_eq!(
            Strictness::from_config_value(Some("loose")).unwrap(),
            Strictness::Loose
        );
        assert_eq!(
            Strictness::from_config_value(Some("permissive")).unwrap(),
            Strictness::Permissive
        );
    }

    #[test]
    fn test_from_config_value_is_case_insensitive() {
        assert_eq!(
            Strictness::from_config_value(Some("STRICT")).unwrap(),
            Strictness::Strict
        );
    }

    #[test]
    fn test_invalid_value_is_an_error_not_a_silent_default() {
        let err = Strictness::from_config_value(Some("banana")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("banana"), "message names the bad value: {msg}");
        assert!(
            msg.contains("strict") && msg.contains("loose") && msg.contains("permissive"),
            "message lists the accepted values: {msg}"
        );
    }

    #[test]
    fn test_token_round_trips_through_from_str() {
        for level in [
            Strictness::Strict,
            Strictness::Loose,
            Strictness::Permissive,
        ] {
            assert_eq!(Strictness::from_str(level.token()).unwrap(), level);
        }
    }
}
