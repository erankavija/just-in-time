//! Closed wire vocabularies: enums whose variants each serialize as one fixed
//! token, declared once through [`wire_vocabulary!`].

/// A token outside a closed wire vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {vocabulary} '{input}' (expected one of: {expected})")]
pub struct UnknownToken {
    /// The vocabulary's type name.
    pub vocabulary: &'static str,
    /// The rejected input, verbatim.
    pub input: String,
    /// The vocabulary's tokens, comma-separated in declaration order.
    pub expected: String,
}

/// Declare a closed wire vocabulary from `Variant = "wire"` entries.
///
/// Emits the enum with a `#[serde(rename = "wire")]` on each variant, a
/// `const fn as_str`, `ALL` in declaration order, an exact-match `FromStr`
/// failing with [`UnknownToken`], and a `Display` printing `as_str`.
/// Attributes and doc comments on the enum and its variants pass through; the
/// enum must derive `Clone`, `Copy`, `Serialize` and `Deserialize`.
macro_rules! wire_vocabulary {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident = $wire:literal
            ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $(
                $(#[$variant_meta])*
                #[serde(rename = $wire)]
                $variant,
            )+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: [Self; [$(stringify!($variant)),+].len()] = [$(Self::$variant),+];

            /// The variant's wire token, as serde writes it.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire,)+
                }
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = $crate::domain::vocabulary::UnknownToken;

            fn from_str(input: &str) -> ::std::result::Result<Self, Self::Err> {
                Self::ALL
                    .into_iter()
                    .find(|variant| variant.as_str() == input)
                    .ok_or_else(|| $crate::domain::vocabulary::UnknownToken {
                        vocabulary: stringify!($name),
                        input: input.to_owned(),
                        expected: Self::ALL.map(Self::as_str).join(", "),
                    })
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

pub(crate) use wire_vocabulary;

#[cfg(test)]
mod tests {
    use schemars::{schema_for, JsonSchema};
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeSet;
    use std::str::FromStr;

    wire_vocabulary! {
        /// Spellings that differ from the variant names in case and separator.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        enum Sample {
            /// Documented, so schemars renders the variant as its own subschema.
            Single = "single",
            Snake = "two_words",
            Kebab = "three-word-token",
        }
    }

    /// Every string a fieldless enum's derived schema admits, from every `enum`
    /// array in the tree (schemars nests documented variants under `oneOf`).
    fn schema_accepted_strings(schema: &serde_json::Value) -> Vec<String> {
        match schema {
            serde_json::Value::Object(map) => map
                .iter()
                .flat_map(|(key, value)| match (key.as_str(), value) {
                    ("enum", serde_json::Value::Array(values)) => values
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect(),
                    _ => schema_accepted_strings(value),
                })
                .collect(),
            serde_json::Value::Array(values) => {
                values.iter().flat_map(schema_accepted_strings).collect()
            }
            _ => Vec::new(),
        }
    }

    #[test]
    fn test_wire_vocabulary_items_agree_with_serde_encoding() {
        Sample::ALL.into_iter().for_each(|variant| {
            assert_eq!(
                serde_json::to_value(variant).unwrap(),
                serde_json::Value::from(variant.as_str()),
                "as_str must equal the serde encoding of {variant:?}"
            );
            assert_eq!(variant.to_string(), variant.as_str());
            assert_eq!(Sample::from_str(variant.as_str()), Ok(variant));

            let token = variant.as_str();
            let variant_name = format!("{variant:?}");
            let near_misses = [
                token.to_uppercase(),
                format!(" {token}"),
                format!("{token} "),
                token[..token.len() - 1].to_string(),
                format!("{token}x"),
                variant_name,
            ];
            near_misses.iter().for_each(|input| {
                let error = Sample::from_str(input).unwrap_err();
                assert_eq!(
                    error.input, *input,
                    "the error must name the rejected input"
                );
                assert_eq!(error.vocabulary, "Sample");
            });
        });
        assert!(Sample::from_str("").is_err());

        let listed: Vec<&str> = Sample::ALL.map(Sample::as_str).to_vec();
        let distinct: BTreeSet<&str> = listed.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            listed.len(),
            "ALL must list each variant once"
        );

        let schema = serde_json::to_value(schema_for!(Sample)).unwrap();
        let derived: BTreeSet<String> = schema_accepted_strings(&schema).into_iter().collect();
        let listed: BTreeSet<String> = distinct.into_iter().map(str::to_string).collect();
        assert_eq!(
            listed, derived,
            "ALL must match the derived schema's variants"
        );
    }
}
