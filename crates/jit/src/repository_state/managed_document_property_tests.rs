//! Property-based round-trip coverage for [`super::render_managed_document`].
//!
//! The generator below produces only claim sets that satisfy
//! `validate_regions`'s acceptance contract by construction: region
//! identities and delimiters are keyed by index (so they are always unique
//! and never alias one another as substrings), and region content and
//! surrounding "outside" bytes are drawn from alphabets that are disjoint
//! from each other and from the delimiter alphabet. That lets the oracle
//! assert the documented round-trip guarantee purely through the public
//! `render_managed_document` API, without ever hitting a rejection path.

use super::{render_managed_document, ManagedDocumentClaim, RegionPlacement};
use proptest::prelude::*;

/// One generated region claim, plus the base-placement decision the
/// generator made for it (needed to assert containment/preservation
/// against the rendered output).
#[derive(Debug, Clone)]
struct GeneratedRegion {
    region_id: String,
    begin: Vec<u8>,
    end: Vec<u8>,
    content: Vec<u8>,
    placement: RegionPlacement,
    present_in_base: bool,
}

impl GeneratedRegion {
    fn claim(&self) -> ManagedDocumentClaim {
        ManagedDocumentClaim::Region {
            owner: format!("owner-{}", self.region_id),
            region_id: self.region_id.clone(),
            begin: self.begin.clone(),
            end: self.end.clone(),
            content: self.content.clone(),
            placement: self.placement,
        }
    }
}

/// Region content: uppercase letters and digits only. Disjoint from the
/// outside-byte alphabet (lowercase and space) and from the delimiter
/// alphabet (which always contains `<`/`>`), so generated content can never
/// be confused with surrounding bytes and can never self-contain a
/// delimiter.
fn content_bytes() -> impl Strategy<Value = Vec<u8>> {
    "[A-Z0-9]{0,16}".prop_map(String::into_bytes)
}

/// Bytes outside every managed region: lowercase letters and space only.
/// Disjoint from `content_bytes` and from the delimiter alphabet, so a
/// match for one of these chunks in the rendered output can only be that
/// literal chunk, never a region's payload.
fn outside_bytes() -> impl Strategy<Value = Vec<u8>> {
    "[a-z ]{0,16}".prop_map(String::into_bytes)
}

/// Per-region random inputs: payload bytes, whether the region requires an
/// existing marker pair (vs. appending when absent), and whether the
/// generator should seed the base document with that marker pair.
fn region_input() -> impl Strategy<Value = (Vec<u8>, bool, bool)> {
    (content_bytes(), any::<bool>(), any::<bool>())
}

/// A base document, the region claims to render against it, and the raw
/// "outside" chunks used to build the base (kept separate so the property
/// test can assert their survival without re-deriving them).
///
/// Regions are keyed by their index in the vector: `region-{index}` /
/// `<<BEGIN{index}>>` / `<<END{index}>>`. Because a decimal index can never
/// be a proper prefix of another index's decimal form followed immediately
/// by the closing `>>`, no two generated delimiters can ever collide or
/// alias as substrings of one another, whatever the region count.
///
/// Regions present in the base carry their delimiter pair inline with
/// nothing between them (`apply_region` discards and replaces whatever sits
/// between the markers, so the filler is irrelevant); absent
/// `AppendIfAbsent` regions carry no markers at all, exercising the append
/// path. An outside chunk separates every region so no two spans ever touch
/// or cross.
fn generated_document() -> impl Strategy<Value = (Vec<u8>, Vec<GeneratedRegion>, Vec<Vec<u8>>)> {
    (1..=6usize).prop_flat_map(|region_count| {
        (
            proptest::collection::vec(outside_bytes(), region_count + 1),
            proptest::collection::vec(region_input(), region_count),
        )
            .prop_map(|(outside_chunks, region_inputs)| {
                let regions: Vec<GeneratedRegion> = region_inputs
                    .into_iter()
                    .enumerate()
                    .map(|(index, (content, require_existing, present_flag))| {
                        let placement = if require_existing {
                            RegionPlacement::RequireExisting
                        } else {
                            RegionPlacement::AppendIfAbsent
                        };
                        // A `RequireExisting` region that is absent from the
                        // base makes `validate_regions` reject the claim
                        // set; force presence so the generator only ever
                        // emits accepted input (REQ-01).
                        let present_in_base =
                            present_flag || placement == RegionPlacement::RequireExisting;
                        GeneratedRegion {
                            region_id: format!("region-{index}"),
                            begin: format!("<<BEGIN{index}>>").into_bytes(),
                            end: format!("<<END{index}>>").into_bytes(),
                            content,
                            placement,
                            present_in_base,
                        }
                    })
                    .collect();

                let mut base = Vec::new();
                for (chunk, region) in outside_chunks.iter().zip(regions.iter()) {
                    base.extend_from_slice(chunk);
                    if region.present_in_base {
                        base.extend_from_slice(&region.begin);
                        base.extend_from_slice(&region.end);
                    }
                }
                base.extend_from_slice(outside_chunks.last().expect("region_count + 1 chunks"));
                (base, regions, outside_chunks)
            })
    })
}

/// Finds `needle` in `haystack` at or after `from`. An empty needle is
/// trivially present at `from`.
fn find_from(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(haystack.len()));
    }
    if from > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

/// Whether `needle` occurs anywhere inside `haystack`. An empty needle is
/// trivially contained.
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

proptest! {
    /// REQ-02: rendering a `validate_regions`-accepted claim set through
    /// `render_managed_document` is idempotent, each region's payload
    /// appears verbatim between its own delimiters, and every byte outside
    /// all claimed regions survives unchanged and in its original relative
    /// order.
    #[test]
    fn prop_render_managed_document_round_trips(
        (base, regions, outside_chunks) in generated_document()
    ) {
        let claims: Vec<ManagedDocumentClaim> =
            regions.iter().map(GeneratedRegion::claim).collect();

        let rendered = render_managed_document(&base, &claims)
            .expect("generator only produces claim sets validate_regions accepts");

        // Idempotence: every claimed region now exists in `rendered`
        // (freshly appended ones included), so re-rendering it against the
        // same claims must reproduce it exactly.
        let rerendered = render_managed_document(&rendered, &claims)
            .expect("every claimed region is present in the document after the first render");
        prop_assert_eq!(&rerendered, &rendered, "rendering must be idempotent");

        // Each region's payload appears verbatim between its own delimiters.
        for region in &regions {
            let begin_at = find_from(&rendered, &region.begin, 0).unwrap_or_else(|| {
                panic!("begin delimiter for '{}' missing after render", region.region_id)
            });
            let body_start = begin_at + region.begin.len();
            let end_at = find_from(&rendered, &region.end, body_start).unwrap_or_else(|| {
                panic!("end delimiter for '{}' missing after render", region.region_id)
            });
            prop_assert!(
                body_start <= end_at,
                "region '{}' delimiters out of order after render",
                region.region_id
            );
            let body = &rendered[body_start..end_at];
            prop_assert!(
                contains_subslice(body, &region.content),
                "region '{}' payload not found verbatim between its delimiters",
                region.region_id
            );
        }

        // Every outside chunk survives unchanged, in its original relative
        // order. The disjoint alphabets guarantee a match can only be that
        // literal chunk, never a region's payload or delimiter.
        let mut cursor = 0usize;
        for chunk in &outside_chunks {
            if chunk.is_empty() {
                continue;
            }
            let at = find_from(&rendered, chunk, cursor)
                .unwrap_or_else(|| panic!("outside bytes {chunk:?} missing after render"));
            cursor = at + chunk.len();
        }
    }
}
