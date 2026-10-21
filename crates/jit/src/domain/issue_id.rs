//! The issue-id matching rule every resolver applies: an input is lowercased and
//! stripped of hyphens, must keep at least [`MIN_ID_PREFIX_LENGTH`] characters,
//! and names each id whose normalised form it prefixes. A full id is the
//! longest prefix of itself, so it matches in any case or hyphenation.

use thiserror::Error;

/// Shortest issue-id prefix any resolver accepts, counted after normalisation.
pub const MIN_ID_PREFIX_LENGTH: usize = 4;

/// Error raised when an id prefix normalises to fewer than
/// [`MIN_ID_PREFIX_LENGTH`] characters.
///
/// The CLI downcasts to this type to classify the failure as an argument error
/// (exit code `2`) and to render an `INVALID_ID_PREFIX` JSON error; the
/// offending prefix is carried for the JSON `details` field.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("Issue ID prefix must be at least {MIN_ID_PREFIX_LENGTH} characters")]
pub struct InvalidIdPrefixError {
    prefix: String,
}

impl InvalidIdPrefixError {
    /// Build an [`InvalidIdPrefixError`] carrying the too-short prefix.
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }

    /// The offending (too-short) prefix.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }
}

/// A requested issue id or id prefix, normalised and length-checked.
///
/// Parsing happens before any candidate is consulted, so a too-short input is
/// refused without reading an issue record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueIdPrefix {
    requested: String,
    normalised: String,
}

impl IssueIdPrefix {
    /// Normalise `requested`.
    ///
    /// # Errors
    ///
    /// [`InvalidIdPrefixError`] when it normalises to fewer than
    /// [`MIN_ID_PREFIX_LENGTH`] characters.
    pub fn parse(requested: &str) -> Result<Self, InvalidIdPrefixError> {
        let normalised = normalise(requested);
        (normalised.chars().count() >= MIN_ID_PREFIX_LENGTH)
            .then(|| Self {
                requested: requested.to_string(),
                normalised,
            })
            .ok_or_else(|| InvalidIdPrefixError::new(requested))
    }

    /// The input as given, for error messages.
    pub fn requested(&self) -> &str {
        &self.requested
    }

    /// Whether this prefix names `id`.
    pub fn matches(&self, id: &str) -> bool {
        normalise(id).starts_with(&self.normalised)
    }
}

fn normalise(id: &str) -> String {
    id.chars()
        .filter(|c| *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const ID: &str = "b53beeba-32f2-4671-a407-2d94e4d13d41";

    #[test]
    fn test_parse_rejects_an_input_normalising_below_the_minimum() {
        let short = &ID.replace('-', "")[..MIN_ID_PREFIX_LENGTH - 1];
        for requested in ["", "----", short, &format!("{short}-")] {
            assert_eq!(
                IssueIdPrefix::parse(requested),
                Err(InvalidIdPrefixError::new(requested))
            );
        }
        assert!(IssueIdPrefix::parse(&ID[..MIN_ID_PREFIX_LENGTH]).is_ok());
    }

    #[test]
    fn test_matches_refuses_an_id_the_prefix_diverges_from() {
        let prefix = IssueIdPrefix::parse("b53beebb").unwrap();
        assert!(!prefix.matches(ID));
        assert!(!IssueIdPrefix::parse(&format!("{ID}0")).unwrap().matches(ID));
    }

    #[test]
    fn test_invalid_id_prefix_error_names_the_minimum_and_carries_the_prefix() {
        let error = InvalidIdPrefixError::new("ab");
        assert!(error
            .to_string()
            .contains(&MIN_ID_PREFIX_LENGTH.to_string()));
        assert_eq!(error.prefix(), "ab");
    }

    proptest! {
        /// Any case and hyphenation of any prefix of at least the minimum
        /// length, the full id included, names the id.
        #[test]
        fn test_matches_any_spelling_of_a_long_enough_prefix(
            len in MIN_ID_PREFIX_LENGTH..=32usize,
            upper in proptest::collection::vec(any::<bool>(), 32),
            hyphens in proptest::collection::vec(0usize..3, 32),
        ) {
            let requested: String = ID
                .replace('-', "")
                .chars()
                .take(len)
                .zip(upper.iter().zip(&hyphens))
                .map(|(c, (upper, hyphens))| {
                    let c = if *upper { c.to_ascii_uppercase() } else { c };
                    format!("{c}{}", "-".repeat(*hyphens))
                })
                .collect();
            prop_assert!(IssueIdPrefix::parse(&requested).unwrap().matches(ID));
        }
    }
}
