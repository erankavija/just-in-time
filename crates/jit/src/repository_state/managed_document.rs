//! Strict byte-preserving managed-document composition.

use super::{CaptureError, RepositoryImage, VirtualPath};
use std::collections::BTreeMap;

/// Placement policy for an absent managed region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RegionPlacement {
    /// Append the delimited region when its markers are absent.
    AppendIfAbsent,
    /// Require exactly one existing marker pair.
    RequireExisting,
}

/// One collected claim against a managed document target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagedDocumentClaim {
    /// Exact base document bytes. At most one base is allowed per target.
    Base {
        /// Stable owner identity.
        owner: String,
        /// Exact base bytes.
        bytes: Vec<u8>,
    },
    /// A byte-delimited region claim.
    Region {
        /// Stable owner identity.
        owner: String,
        /// Stable region identity.
        region_id: String,
        /// Exact begin delimiter.
        begin: Vec<u8>,
        /// Exact end delimiter.
        end: Vec<u8>,
        /// Exact managed body, excluding delimiters.
        content: Vec<u8>,
        /// Explicit absent-marker policy.
        placement: RegionPlacement,
    },
}

/// Compose every target only after all claims have been collected.
pub(crate) fn compose_managed_documents(
    image: &RepositoryImage,
    claims: impl IntoIterator<Item = (VirtualPath, ManagedDocumentClaim)>,
) -> Result<BTreeMap<VirtualPath, Vec<u8>>, ManagedDocumentError> {
    let grouped = claims.into_iter().try_fold(
        BTreeMap::<VirtualPath, Vec<ManagedDocumentClaim>>::new(),
        |mut grouped, (target, claim)| {
            image.layout().ensure_canonical(&target)?;
            grouped.entry(target).or_default().push(claim);
            Ok::<_, ManagedDocumentError>(grouped)
        },
    )?;
    grouped
        .into_iter()
        .map(|(target, claims)| {
            let existing = image.file_bytes(&target)?.unwrap_or_default();
            render_managed_document(existing, &claims).map(|bytes| (target, bytes))
        })
        .collect()
}

/// Render one target while preserving every byte outside claimed regions.
pub fn render_managed_document(
    existing: &[u8],
    claims: &[ManagedDocumentClaim],
) -> Result<Vec<u8>, ManagedDocumentError> {
    let bases = claims
        .iter()
        .filter_map(|claim| match claim {
            ManagedDocumentClaim::Base { owner, bytes } => Some((owner, bytes)),
            ManagedDocumentClaim::Region { .. } => None,
        })
        .collect::<Vec<_>>();
    if bases.len() > 1 {
        return Err(ManagedDocumentError::CompetingBaseClaims(
            bases.iter().map(|(owner, _)| (*owner).clone()).collect(),
        ));
    }
    let base = bases
        .first()
        .map_or(existing, |(_, bytes)| bytes.as_slice());
    let regions = claims
        .iter()
        .filter_map(|claim| match claim {
            ManagedDocumentClaim::Region {
                owner,
                region_id,
                begin,
                end,
                content,
                placement,
            } => Some(RegionClaim {
                owner,
                region_id,
                begin,
                end,
                content,
                placement: *placement,
            }),
            ManagedDocumentClaim::Base { .. } => None,
        })
        .collect::<Vec<_>>();
    let ordered = validate_regions(base, &regions)?;
    ordered
        .into_iter()
        .try_fold(base.to_vec(), |bytes, region| apply_region(bytes, region))
}

#[derive(Debug, Clone, Copy)]
struct RegionClaim<'a> {
    owner: &'a str,
    region_id: &'a str,
    begin: &'a [u8],
    end: &'a [u8],
    content: &'a [u8],
    placement: RegionPlacement,
}

#[derive(Debug, Clone, Copy)]
struct LocatedRegion<'a> {
    claim: RegionClaim<'a>,
    begin: Option<usize>,
    end: Option<usize>,
}

fn validate_regions<'a>(
    base: &[u8],
    regions: &'a [RegionClaim<'a>],
) -> Result<Vec<LocatedRegion<'a>>, ManagedDocumentError> {
    let mut identities = BTreeMap::<&str, &str>::new();
    let mut delimiter_owners = BTreeMap::<Vec<u8>, &str>::new();
    let located = regions
        .iter()
        .copied()
        .map(|claim| {
            match identities.insert(claim.region_id, claim.owner) {
                Some(owner) if owner == claim.owner => {
                    return Err(ManagedDocumentError::DuplicateRegionIdentity(
                        claim.region_id.to_string(),
                    ));
                }
                Some(owner) => {
                    return Err(ManagedDocumentError::MultiplyOwnedRegion {
                        region_id: claim.region_id.to_string(),
                        owners: vec![owner.to_string(), claim.owner.to_string()],
                    });
                }
                None => {}
            }
            if claim.begin.is_empty() || claim.end.is_empty() || claim.begin == claim.end {
                return Err(ManagedDocumentError::AmbiguousDelimiters(
                    claim.region_id.to_string(),
                ));
            }
            for delimiter in [claim.begin, claim.end] {
                if let Some(owner) = delimiter_owners.insert(delimiter.to_vec(), claim.region_id) {
                    return Err(ManagedDocumentError::AmbiguousDelimiterOwnership {
                        delimiter: String::from_utf8_lossy(delimiter).into_owned(),
                        regions: vec![owner.to_string(), claim.region_id.to_string()],
                    });
                }
            }
            if !find_all(claim.content, claim.begin).is_empty()
                || !find_all(claim.content, claim.end).is_empty()
            {
                return Err(ManagedDocumentError::SelfContainingContent(
                    claim.region_id.to_string(),
                ));
            }
            let begins = find_all(base, claim.begin);
            let ends = find_all(base, claim.end);
            let (begin, end) = match (begins.as_slice(), ends.as_slice()) {
                ([], []) if claim.placement == RegionPlacement::AppendIfAbsent => (None, None),
                ([], []) => {
                    return Err(ManagedDocumentError::RequiredRegionAbsent(
                        claim.region_id.to_string(),
                    ));
                }
                ([], _) | (_, []) => {
                    return Err(ManagedDocumentError::PartialRegion(
                        claim.region_id.to_string(),
                    ));
                }
                ([begin], [end]) if begin < end => (Some(*begin), Some(*end + claim.end.len())),
                ([begin], [end]) if end < begin => {
                    return Err(ManagedDocumentError::ReversedRegion(
                        claim.region_id.to_string(),
                    ));
                }
                _ => {
                    return Err(ManagedDocumentError::DuplicateMarkers(
                        claim.region_id.to_string(),
                    ));
                }
            };
            Ok(LocatedRegion { claim, begin, end })
        })
        .collect::<Result<Vec<_>, _>>()?;

    for (index, left) in located.iter().enumerate() {
        for right in &located[index + 1..] {
            if let (Some(left_begin), Some(left_end), Some(right_begin), Some(right_end)) =
                (left.begin, left.end, right.begin, right.end)
            {
                let crossing = (left_begin < right_begin
                    && right_begin < left_end
                    && left_end < right_end)
                    || (right_begin < left_begin && left_begin < right_end && right_end < left_end);
                if crossing {
                    return Err(ManagedDocumentError::CrossingRegions {
                        first: left.claim.region_id.to_string(),
                        second: right.claim.region_id.to_string(),
                    });
                }
            }
        }
    }

    let mut ordered = located;
    ordered.sort_by(|left, right| match (left.begin, right.begin) {
        (Some(left_at), Some(right_at)) => left_at
            .cmp(&right_at)
            .then_with(|| right.end.cmp(&left.end)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.claim.region_id.cmp(right.claim.region_id),
    });
    Ok(ordered)
}

fn apply_region(
    mut bytes: Vec<u8>,
    located: LocatedRegion<'_>,
) -> Result<Vec<u8>, ManagedDocumentError> {
    let claim = located.claim;
    // Whether the claim resolved to a region in the ORIGINAL base (validated by
    // `validate_regions`), as opposed to being absent then. Positions may have
    // shifted under earlier splices, but this presence fact is fixed.
    let was_present = located.begin.is_some();
    let begins = find_all(&bytes, claim.begin);
    let ends = find_all(&bytes, claim.end);
    match (begins.as_slice(), ends.as_slice()) {
        // Absent in the original base under an append policy: append at the tail.
        ([], []) if !was_present && claim.placement == RegionPlacement::AppendIfAbsent => {
            if !bytes.is_empty() && !bytes.ends_with(b"\n") {
                bytes.push(b'\n');
            }
            if !bytes.is_empty() && !bytes.ends_with(b"\n\n") {
                bytes.push(b'\n');
            }
            append_region(&mut bytes, claim);
            Ok(bytes)
        }
        // Present in the original base but gone now: an already-applied outer
        // region's replacement removed this claimed child. Reject the removal
        // rather than silently re-appending the child as a top-level sibling
        // (plan §2: "reject an outer replacement that removes a claimed child").
        ([], []) if was_present => Err(ManagedDocumentError::ClaimedChildRemoved(
            claim.region_id.to_string(),
        )),
        // Absent under a require-existing policy.
        ([], []) => Err(ManagedDocumentError::RequiredRegionAbsent(
            claim.region_id.to_string(),
        )),
        ([begin], [end]) if begin < end => {
            let body_start = *begin + claim.begin.len();
            let mut rendered = Vec::with_capacity(bytes.len() + claim.content.len());
            rendered.extend_from_slice(&bytes[..body_start]);
            rendered.push(b'\n');
            rendered.extend_from_slice(claim.content);
            if !claim.content.ends_with(b"\n") {
                rendered.push(b'\n');
            }
            rendered.extend_from_slice(&bytes[*end..]);
            Ok(rendered)
        }
        _ => Err(ManagedDocumentError::StructureChangedDuringComposition(
            claim.region_id.to_string(),
        )),
    }
}

fn append_region(bytes: &mut Vec<u8>, claim: RegionClaim<'_>) {
    bytes.extend_from_slice(claim.begin);
    bytes.push(b'\n');
    bytes.extend_from_slice(claim.content);
    if !claim.content.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(claim.end);
    bytes.push(b'\n');
}

/// The bytes one region's delimiters enclose in `document`, or `None` when the
/// document does not hold exactly that region.
///
/// This is the read side of the splice [`apply_region`] performs, so an owner
/// asking what the repository currently holds for its region reads it back
/// through the same delimiter rule composition wrote it under: exactly one
/// begin and one end delimiter, in that order. A document carrying neither,
/// only one, or several answers `None`, because no single region is named.
///
/// The returned body still carries the newline the splice inserts after the
/// begin delimiter; only the delimiters themselves are excluded.
pub(crate) fn region_body<'a>(document: &'a [u8], begin: &[u8], end: &[u8]) -> Option<&'a [u8]> {
    let begins = find_all(document, begin);
    let ends = find_all(document, end);
    match (begins.as_slice(), ends.as_slice()) {
        ([at], [to]) if at + begin.len() <= *to => document.get(at + begin.len()..*to),
        _ => None,
    }
}

fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    (0..=haystack.len().saturating_sub(needle.len()))
        .filter(|index| haystack.get(*index..*index + needle.len()) == Some(needle))
        .collect()
}

/// Invalid or ambiguous managed-document claim structure.
#[derive(Debug, thiserror::Error)]
pub enum ManagedDocumentError {
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Layout(#[from] super::RepositoryLayoutError),
    #[error("competing base claims from {0:?}")]
    CompetingBaseClaims(Vec<String>),
    #[error("duplicate managed region identity '{0}'")]
    DuplicateRegionIdentity(String),
    #[error("managed region '{region_id}' is owned by several producers: {owners:?}")]
    MultiplyOwnedRegion {
        region_id: String,
        owners: Vec<String>,
    },
    #[error("managed region '{0}' has ambiguous delimiters")]
    AmbiguousDelimiters(String),
    #[error("delimiter '{delimiter}' belongs to several regions: {regions:?}")]
    AmbiguousDelimiterOwnership {
        delimiter: String,
        regions: Vec<String>,
    },
    #[error("managed content for '{0}' contains its own delimiter")]
    SelfContainingContent(String),
    #[error("required managed region '{0}' is absent")]
    RequiredRegionAbsent(String),
    #[error("managed region '{0}' has only one delimiter")]
    PartialRegion(String),
    #[error("managed region '{0}' has reversed delimiters")]
    ReversedRegion(String),
    #[error("managed region '{0}' has duplicate delimiters")]
    DuplicateMarkers(String),
    #[error("managed regions '{first}' and '{second}' cross")]
    CrossingRegions { first: String, second: String },
    #[error("managed region '{0}' changed containment during composition")]
    StructureChangedDuringComposition(String),
    #[error("outer managed region removed claimed child region '{0}'")]
    ClaimedChildRemoved(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(
        owner: &str,
        id: &str,
        begin: &[u8],
        end: &[u8],
        content: &[u8],
    ) -> ManagedDocumentClaim {
        ManagedDocumentClaim::Region {
            owner: owner.into(),
            region_id: id.into(),
            begin: begin.into(),
            end: end.into(),
            content: content.into(),
            placement: RegionPlacement::RequireExisting,
        }
    }

    #[test]
    fn test_managed_document_accepts_distinct_nested_regions_and_preserves_unmanaged_bytes() {
        let existing = b"prefix\n<O>\nouter <I>old</I> tail\n</O>\nsuffix\n";
        let claims = [
            region(
                "profile",
                "outer",
                b"<O>",
                b"</O>",
                b"outer <I>old</I> tail",
            ),
            region("project", "inner", b"<I>", b"</I>", b"new"),
        ];
        let rendered = render_managed_document(existing, &claims).unwrap();
        assert_eq!(
            rendered,
            b"prefix\n<O>\nouter <I>\nnew\n</I> tail\n</O>\nsuffix\n"
        );
    }

    #[test]
    fn test_managed_document_rejects_duplicate_reversed_crossing_and_multiply_owned() {
        let duplicate = [
            region("one", "same", b"<A>", b"</A>", b"x"),
            region("one", "same", b"<B>", b"</B>", b"y"),
        ];
        assert!(matches!(
            render_managed_document(b"<A>x</A><B>y</B>", &duplicate),
            Err(ManagedDocumentError::DuplicateRegionIdentity(_))
        ));
        assert!(matches!(
            render_managed_document(b"</A>x<A>", &[region("one", "a", b"<A>", b"</A>", b"x")]),
            Err(ManagedDocumentError::ReversedRegion(_))
        ));
        let crossing = [
            region("one", "a", b"<A>", b"</A>", b"x"),
            region("two", "b", b"<B>", b"</B>", b"y"),
        ];
        assert!(matches!(
            render_managed_document(b"<A><B></A></B>", &crossing),
            Err(ManagedDocumentError::CrossingRegions { .. })
        ));
        let multiply_owned = [
            region("one", "same", b"<A>", b"</A>", b"x"),
            region("two", "same", b"<B>", b"</B>", b"y"),
        ];
        assert!(matches!(
            render_managed_document(b"<A>x</A><B>y</B>", &multiply_owned),
            Err(ManagedDocumentError::MultiplyOwnedRegion { .. })
        ));
    }

    #[test]
    fn test_managed_document_rejects_partial_and_competing_base_claims() {
        assert!(matches!(
            render_managed_document(
                b"<A>missing end",
                &[region("one", "a", b"<A>", b"</A>", b"x")]
            ),
            Err(ManagedDocumentError::PartialRegion(_))
        ));
        let bases = [
            ManagedDocumentClaim::Base {
                owner: "one".into(),
                bytes: b"one".to_vec(),
            },
            ManagedDocumentClaim::Base {
                owner: "two".into(),
                bytes: b"two".to_vec(),
            },
        ];
        assert!(matches!(
            render_managed_document(b"", &bases),
            Err(ManagedDocumentError::CompetingBaseClaims(_))
        ));
    }

    /// The reader is the inverse of the splice: what composition writes into a
    /// document is what an owner reads back out of it, and a document naming no
    /// single region answers with nothing rather than guessing at one.
    #[test]
    fn test_region_body_reads_back_the_content_the_splice_published() {
        let published = render_managed_document(
            b"prefix\n<B>\nstale\n</B>\nsuffix\n",
            &[region("profile", "guidance", b"<B>", b"</B>", b"managed\n")],
        )
        .expect("the region composes over its existing markers");

        assert_eq!(
            region_body(&published, b"<B>", b"</B>"),
            Some(b"\nmanaged\n".as_slice()),
            "the body carries the content and the newline the splice inserts"
        );
        for unnamed in [
            b"no markers at all".as_slice(),
            b"<B> only a begin".as_slice(),
            b"only an end </B>".as_slice(),
            b"<B>first</B> and <B>second</B>".as_slice(),
        ] {
            assert_eq!(region_body(unnamed, b"<B>", b"</B>"), None, "{unnamed:?}");
        }
    }

    #[test]
    fn test_outer_region_removing_claimed_child_is_rejected() {
        // The inner child is PRESENT in the base but nested inside the outer
        // region. The outer's replacement content omits the child's markers, so
        // applying it deletes the child. An AppendIfAbsent child must NOT be
        // silently re-appended as a top-level sibling — the removal is rejected
        // (plan §2: reject an outer replacement that removes a claimed child).
        let existing = b"<O>\nouter <I>old</I> tail\n</O>\n";
        let outer = ManagedDocumentClaim::Region {
            owner: "profile".into(),
            region_id: "outer".into(),
            begin: b"<O>".to_vec(),
            end: b"</O>".to_vec(),
            content: b"outer tail".to_vec(),
            placement: RegionPlacement::RequireExisting,
        };
        let inner = ManagedDocumentClaim::Region {
            owner: "project".into(),
            region_id: "inner".into(),
            begin: b"<I>".to_vec(),
            end: b"</I>".to_vec(),
            content: b"new".to_vec(),
            placement: RegionPlacement::AppendIfAbsent,
        };
        assert!(matches!(
            render_managed_document(existing, &[outer, inner]),
            Err(ManagedDocumentError::ClaimedChildRemoved(_))
        ));
    }
}

// Include property-based round-trip coverage.
#[cfg(test)]
#[path = "managed_document_property_tests.rs"]
mod property_tests;
