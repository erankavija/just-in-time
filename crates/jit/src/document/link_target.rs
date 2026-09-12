//! Classify document link destinations before treating them as repository paths.
//!
//! Foreign schemes such as `mailto:`, `tel:`, `data:`, and `file:` are not
//! repository paths. This also excludes a demangled Rust symbol such as
//! `memchr::arch::x86_64::memchr::count_raw::find_avx2`, which CommonMark can
//! parse as an absolute-URI autolink with the `memchr` scheme.

/// A document reference classified as an external URL or local path.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LinkTarget<'a> {
    /// An http/https URL, recorded as an external reference.
    External(&'a str),
    /// A repository-relative or root-relative path.
    Local(&'a str),
}

/// The RFC 3986 scheme a reference carries, when it carries one.
pub(crate) fn reference_scheme(reference: &str) -> Option<&str> {
    let colon = reference.find(':')?;
    let scheme = reference.get(..colon)?;
    let mut characters = scheme.bytes();

    if !characters
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic())
        || !characters
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
    {
        return None;
    }

    Some(scheme)
}

/// Whether a reference names an http/https URL.
pub(crate) fn is_external_reference(reference: &str) -> bool {
    reference_scheme(reference).is_some_and(|scheme| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    })
}

/// Classify one raw destination, or `None` when it denotes no target.
pub(crate) fn classify_link_target(reference: &str) -> Option<LinkTarget<'_>> {
    let reference = reference.trim();
    if reference.is_empty() || reference.starts_with('#') {
        return None;
    }

    let reference = reference
        .split_once('#')
        .map_or(reference, |(path, _)| path);
    if reference.is_empty() {
        return None;
    }

    match reference_scheme(reference) {
        None => Some(LinkTarget::Local(reference)),
        Some(_) if is_external_reference(reference) => Some(LinkTarget::External(reference)),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reference_scheme_present() {
        assert_eq!(reference_scheme("http://example.com"), Some("http"));
    }

    #[test]
    fn test_reference_scheme_absent() {
        assert_eq!(reference_scheme("docs/guide.md"), None);
    }

    #[test]
    fn test_reference_scheme_invalid_first_character() {
        assert_eq!(reference_scheme("_scheme:value"), None);
    }

    #[test]
    fn test_reference_scheme_digit_leading() {
        assert_eq!(reference_scheme("1scheme:value"), None);
    }

    #[test]
    fn test_reference_scheme_allows_scheme_punctuation() {
        assert_eq!(reference_scheme("a+b-c.d:value"), Some("a+b-c.d"));
    }

    #[test]
    fn test_classify_link_target_anchor_only() {
        assert_eq!(classify_link_target("#section"), None);
    }

    #[test]
    fn test_classify_link_target_empty() {
        assert_eq!(classify_link_target("  \t"), None);
    }

    #[test]
    fn test_classify_link_target_strips_fragment() {
        assert_eq!(
            classify_link_target(" docs/guide.md#section "),
            Some(LinkTarget::Local("docs/guide.md"))
        );
    }

    #[test]
    fn test_classify_link_target_without_scheme_is_local() {
        assert_eq!(
            classify_link_target("docs/guide.md"),
            Some(LinkTarget::Local("docs/guide.md"))
        );
    }

    #[test]
    fn test_classify_link_target_http_and_https_are_external() {
        assert_eq!(
            classify_link_target("http://example.com/image.png"),
            Some(LinkTarget::External("http://example.com/image.png"))
        );
        assert_eq!(
            classify_link_target("HTTPS://example.com/image.png"),
            Some(LinkTarget::External("HTTPS://example.com/image.png"))
        );
    }

    #[test]
    fn test_classify_link_target_mailto_is_not_local() {
        assert_eq!(classify_link_target("mailto:user@example.com"), None);
    }

    #[test]
    fn test_classify_link_target_skips_a_demangled_rust_symbol() {
        assert_eq!(
            classify_link_target("memchr::arch::x86_64::memchr::count_raw::find_avx2"),
            None
        );
    }
}
