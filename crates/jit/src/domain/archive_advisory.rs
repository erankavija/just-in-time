//! Advisory citations beside an archive plan: where live files cite each
//! relocating artifact. The report reads files outside the plan's captured
//! image, so no plan or write derives from it.

use crate::document::{LinkType, LinkValidator};
use crate::domain::artifact_execution::ArchiveExecutionResult;
use crate::domain::artifact_plan::{ArtifactAction, ArtifactPlan};
use crate::domain::relative_path::RootRelativePath;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Where a scanned file cites a source: its normalized repository-relative
/// path, with the citation's 1-based line and character column. Its wire form
/// is `<citing path>:<line>:<column>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CitationLocation {
    pub citing: String,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for CitationLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.citing, self.line, self.column)
    }
}

impl std::str::FromStr for CitationLocation {
    type Err = String;

    fn from_str(location: &str) -> Result<Self, Self::Err> {
        let mut parts = location.rsplitn(3, ':');
        let column = parts.next().and_then(|part| part.parse().ok());
        let line = parts.next().and_then(|part| part.parse().ok());
        match (parts.next(), line, column) {
            (Some(citing), Some(line), Some(column)) => Ok(Self {
                citing: citing.to_string(),
                line,
                column,
            }),
            _ => Err(format!(
                "`{location}` is not `<citing path>:<line>:<column>`"
            )),
        }
    }
}

impl Serialize for CitationLocation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for CitationLocation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for CitationLocation {
    fn schema_name() -> String {
        "CitationLocation".to_string()
    }

    fn json_schema(generator: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        String::json_schema(generator)
    }
}

/// Cited source to the locations citing it, built by one [`CitationScanner`]
/// pass over the configured citation roots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CitationIndex(BTreeMap<String, BTreeSet<CitationLocation>>);

impl CitationIndex {
    /// The locations citing `source`, in location order.
    pub fn citations<'a>(&'a self, source: &str) -> impl Iterator<Item = &'a CitationLocation> {
        self.0.get(source).into_iter().flatten()
    }

    /// The advisory report for `plans`, by lookup of each artifact's source.
    ///
    /// Only a relocating artifact is reported: one whose source stays where it
    /// is leaves every citation of that path resolvable.
    pub fn advise<'a>(&self, plans: impl IntoIterator<Item = &'a ArtifactPlan>) -> ArchiveAdvisory {
        ArchiveAdvisory {
            citations: plans
                .into_iter()
                .flat_map(|plan| plan.artifacts())
                .filter(|artifact| artifact.action() == ArtifactAction::Move)
                .map(|artifact| artifact.source())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|artifact| ArtifactCitations {
                    artifact: artifact.to_string(),
                    locations: self.citations(artifact).cloned().collect(),
                })
                .filter(|citations| !citations.locations.is_empty())
                .collect(),
        }
    }
}

/// Every artifact source of `plans`: the pattern set one index scan covers.
pub fn planned_sources<'a>(plans: impl IntoIterator<Item = &'a ArtifactPlan>) -> BTreeSet<String> {
    plans
        .into_iter()
        .flat_map(|plan| plan.artifacts())
        .map(|artifact| artifact.source().to_string())
        .collect()
}

/// Finds citations of a fixed set of candidate sources one file at a time, so
/// a scan holds only the file it is reading.
///
/// Each file's text is read once and searched two ways. One overlapping
/// multi-pattern search finds every occurrence of every source, each judged on
/// its own line by `is_whole_path_citation`. Per source, occurrences are
/// examined left to right, resuming past the end of each one examined. That
/// loses no citation: an occurrence starting inside another is preceded by the
/// cited path's own opening characters, which name path content, so
/// `is_whole_path_citation` rejects it. The text's Markdown links, from
/// [`LinkValidator::scan_links`], are resolved from the citing file's
/// directory through [`RootRelativePath::resolve_link`], which finds a link
/// such as `[plan](plan.md)` that never spells the source; a link whose
/// destination text already holds a whole-path citation of the source it
/// resolves to is not counted again. Columns are counted in characters, so
/// text in any script is located where its reader sees it.
pub struct CitationScanner {
    sources: Vec<String>,
    automaton: aho_corasick::AhoCorasick,
}

impl CitationScanner {
    /// A scanner for `sources`; an empty source cites nothing and is dropped.
    pub fn new(
        sources: impl IntoIterator<Item = String>,
    ) -> Result<Self, aho_corasick::BuildError> {
        let sources = sources
            .into_iter()
            .filter(|source| !source.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        Ok(Self {
            automaton: aho_corasick::AhoCorasick::new(&sources)?,
            sources,
        })
    }

    /// Record in `index` every citation `bytes`, read from the
    /// repository-relative path `citing`, makes of a candidate source. Bytes
    /// that are not valid UTF-8 contribute nothing.
    pub fn scan(&self, citing: &str, bytes: &[u8], index: &mut CitationIndex) {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return;
        };
        let whole_paths = self.whole_path_citations(text);
        let directory = RootRelativePath::parse(citing)
            .ok()
            .and_then(|path| path.split_leaf().map(|(parent, _)| parent))
            .unwrap_or(RootRelativePath::Root);
        let linked = LinkValidator::scan_links(text)
            .into_iter()
            .filter_map(|link| {
                let resolved = match link.link_type {
                    LinkType::RootRelative => RootRelativePath::Root.resolve_link(&link.target),
                    LinkType::Relative => directory.resolve_link(&link.target),
                    LinkType::Anchor => None,
                }?;
                let source = self.sources.binary_search(&resolved).ok()?;
                (!whole_paths
                    .iter()
                    .any(|(cited, offset, ..)| *cited == source && link.span.contains(offset)))
                .then_some((source, link.line_number, link.column))
            })
            .collect::<Vec<_>>();
        whole_paths
            .iter()
            .map(|(source, _, line, column)| (*source, *line, *column))
            .chain(linked)
            .for_each(|(source, line, column)| {
                index
                    .0
                    .entry(self.sources[source].clone())
                    .or_default()
                    .insert(CitationLocation {
                        citing: citing.to_string(),
                        line,
                        column,
                    });
            });
    }

    /// Every whole-path citation in `text`, as its source's index, byte offset,
    /// 1-based line and character column.
    fn whole_path_citations(&self, text: &str) -> Vec<(usize, usize, usize, usize)> {
        let mut occurrences = self
            .automaton
            .find_overlapping_iter(text)
            .map(|found| (found.start(), found.pattern().as_usize()))
            .collect::<Vec<_>>();
        occurrences.sort_unstable();
        let mut examined_until = vec![0; self.sources.len()];
        let mut line = TextLine::default();
        let mut citations = Vec::new();
        for (offset, index) in occurrences {
            let source = &self.sources[index];
            if offset < examined_until[index] {
                continue;
            }
            examined_until[index] = offset + source.len();
            line.advance_to(text, offset);
            let content = line.content(text);
            let within = offset - line.start;
            if within + source.len() <= content.len()
                && is_whole_path_citation(content, within, source)
            {
                citations.push((
                    index,
                    offset,
                    line.number,
                    content[..within].chars().count() + 1,
                ));
            }
        }
        citations
    }
}

/// The line holding the current scan position, advanced monotonically.
///
/// Lines split as [`str::lines`] splits them: at `\n`, with a `\r` before it
/// belonging to the line ending rather than the content.
#[derive(Default)]
struct TextLine {
    start: usize,
    end: usize,
    number: usize,
}

impl TextLine {
    /// Move to the line holding byte `offset`, which never precedes this line.
    fn advance_to(&mut self, text: &str, offset: usize) {
        if self.number > 0 && offset <= self.end {
            return;
        }
        let skipped = &text[self.start..offset];
        self.number = self.number.max(1) + skipped.matches('\n').count();
        self.start += skipped.rfind('\n').map_or(0, |last| last + 1);
        self.end = text[self.start..]
            .find('\n')
            .map_or(text.len(), |end| self.start + end);
    }

    fn content<'a>(&self, text: &'a str) -> &'a str {
        let line = &text[self.start..self.end];
        match self.end < text.len() {
            true => line.strip_suffix('\r').unwrap_or(line),
            false => line,
        }
    }
}

/// Whether the occurrence of `source` at byte `offset` in `line` cites that path
/// whole, rather than sitting inside a longer one.
///
/// A longer path reaches the occurrence through a directory name, whether it
/// stands above the cited path (`dev/archive/8e071e18/active/plan.md`, the
/// destination every relocation publishes) or merely ends with the cited path's
/// first segment (`mydev/active/plan.md`), and it extends past the occurrence
/// through a longer name (`dev/active/plan.mdx`). Repository filenames can use
/// nearly every non-NUL, non-slash character, so a small allow-list of filename
/// characters cannot reliably find that boundary. Instead, an occurrence is a
/// citation only when prose or code syntax delimits it.
///
/// Punctuation carrying no name leaves the citation whole, which is how the
/// citations prose and code actually write are kept: a quote or bracket around
/// the path, the `-` of a shell default (`${OUT:-dev/active/plan.md}`), a
/// sentence period after it, and a relative `./` or `../` before it, which adds
/// no directory and marks a citation the same relocation breaks. A period is
/// only sentence punctuation when what follows cannot continue a filename.
fn is_whole_path_citation(line: &str, offset: usize, source: &str) -> bool {
    let prefix = &line[..offset];
    let suffix = &line[offset + source.len()..];
    citation_start(prefix).is_some_and(|start| match start {
        CitationStart::Delimited => is_citation_end(suffix),
        CitationStart::ShellDefault => is_shell_default_end(suffix),
    })
}

/// The syntax that introduces a standalone path citation.
#[derive(Clone, Copy)]
enum CitationStart {
    Delimited,
    ShellDefault,
}

/// Whether text before a path finishes with syntax that introduces a citation.
///
/// Relative `./` and `../` prefixes still name the same repository-relative
/// artifact. A shell default is a citation only when its valid parameter name,
/// opening `${`, and closing `}` prove the source occupies the whole default.
fn citation_start(prefix: &str) -> Option<CitationStart> {
    shell_default_start(prefix)
        .then_some(CitationStart::ShellDefault)
        .or_else(|| relative_prefix_start(prefix))
        .or_else(|| {
            (prefix.is_empty() || prefix.chars().last().is_some_and(is_citation_delimiter))
                .then_some(CitationStart::Delimited)
        })
}

/// Whether `prefix` ends with a standalone `${NAME:-` shell-default introducer.
fn shell_default_start(prefix: &str) -> bool {
    prefix.strip_suffix(":-").is_some_and(|before_default| {
        before_default
            .rsplit_once("${")
            .is_some_and(|(before_open, name)| {
                is_shell_variable_name(name)
                    && (before_open.is_empty()
                        || before_open
                            .chars()
                            .last()
                            .is_some_and(is_citation_delimiter))
            })
    })
}

/// Whether `name` is a portable shell variable name.
fn is_shell_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

/// Whether `suffix` closes a shell default without extending its value.
fn is_shell_default_end(suffix: &str) -> bool {
    suffix.strip_prefix('}').is_some_and(|after_default| {
        !after_default.starts_with('}') && is_citation_end(after_default)
    })
}

/// Whether text after a path starts with syntax that closes a citation.
fn is_citation_end(suffix: &str) -> bool {
    suffix.is_empty()
        || suffix.chars().next().is_some_and(is_citation_delimiter)
        || is_sentence_period(suffix)
}

/// Whether `suffix` begins with a sentence period rather than a filename extension.
fn is_sentence_period(suffix: &str) -> bool {
    suffix.strip_prefix('.').is_some_and(|after_period| {
        after_period.is_empty()
            || after_period
                .chars()
                .next()
                .is_some_and(is_sentence_period_follower)
    })
}

/// Whether `character` can follow sentence-ending punctuation without extending a path.
fn is_sentence_period_follower(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\'' | '"' | '`' | ')' | ']' | '}' | '>')
}

/// Whether `prefix` ends in one or more repository-relative path introducers.
fn relative_prefix_start(prefix: &str) -> Option<CitationStart> {
    ["../", "./"].into_iter().find_map(|relative| {
        prefix.strip_suffix(relative).and_then(|before| {
            (!before.ends_with('.'))
                .then_some(())
                .and_then(|()| citation_start(before))
        })
    })
}

/// Whether `character` delimits a path citation in prose or code.
///
/// This intentionally lists syntax rather than filename characters. A character
/// not listed here is allowed in a repository filename and therefore keeps an
/// adjacent source occurrence inside a longer path.
fn is_citation_delimiter(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '\'' | '"'
                | '`'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | ','
                | ';'
                | '!'
                | '?'
        )
}

/// The advisory section of an archive preview, candidate listing or execution.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveAdvisory {
    /// Each relocating artifact a scanned file cites, in source order.
    pub citations: Vec<ArtifactCitations>,
}

/// The scanned citations of one relocating artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArtifactCitations {
    /// The artifact's normalized source path.
    pub artifact: String,
    /// Every citation of it, in location order.
    pub locations: Vec<CitationLocation>,
}

/// An archive preview: the plan beside its advisory section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ArchivePreview {
    #[serde(flatten)]
    pub plan: ArtifactPlan,
    pub advisory: ArchiveAdvisory,
}

/// An archive execution's result beside its advisory section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ArchiveExecutionReport {
    #[serde(flatten)]
    pub result: ArchiveExecutionResult,
    pub advisory: ArchiveAdvisory,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::artifact_classifier::{ArchiveLayout, ArtifactClassificationPolicy};
    use crate::domain::artifact_plan::{
        ArtifactPlanEntry, ArtifactVersion, PlanTarget, PolicyStatus,
    };
    use proptest::prelude::*;

    /// The path whose relocation the citation fixture below would break.
    const CITED_SOURCE: &str = "dev/active/plan.md";

    /// Scanned file text covering the citation kinds the in-content census in
    /// `dev/archive/8e071e18-dev-artifact-layout/active/8e071e18-investigation.md`
    /// measured: a production Rust module
    /// doc comment, an adopter-facing inline code span, a shell-script comment
    /// beside an executable default, and one file citing the path twice on a
    /// single line. The last entry is one of the census's verified
    /// non-citations, an illustrative JSON sample naming a different path.
    fn citation_fixture() -> [(&'static str, &'static str); 5] {
        [
            (
                "crates/jit/src/storage/claim_coordinator.rs",
                "//! Claim coordination.\n//! See design doc: `dev/active/plan.md` - \"Claim Acquisition\"\n",
            ),
            (
                "docs/tutorials/parallel-work-worktrees.md",
                "## Further reading\n\n- Design document: `dev/active/plan.md`\n",
            ),
            (
                "scripts/benchmark-rust-build.sh",
                "#!/usr/bin/env bash\n# Outputs (default dev/active/plan.md)\nOUT=\"${BENCH_OUT:-dev/active/plan.md}\"\n",
            ),
            (
                "dev/studies/tooling.md",
                "Both `dev/active/plan.md` and `dev/active/plan.md` cover it.\n",
            ),
            (
                "docs/concepts/core-model.md",
                "An illustrative sample: {\"path\": \"dev/design/auth-design.md\"}\n",
            ),
        ]
    }

    /// The index one scan of `files` builds for `sources`.
    fn citations(sources: &[&str], files: &[(&str, &str)]) -> CitationIndex {
        let scanner = CitationScanner::new(sources.iter().map(|source| source.to_string()))
            .expect("fixture sources build a scanner");
        let mut index = CitationIndex::default();
        for (path, text) in files {
            scanner.scan(path, text.as_bytes(), &mut index);
        }
        index
    }

    /// Every location the index records for `source`, as `(citing, line, column)`.
    fn located(index: &CitationIndex, source: &str) -> Vec<(String, usize, usize)> {
        index
            .citations(source)
            .map(|location| (location.citing.clone(), location.line, location.column))
            .collect()
    }

    fn cited_located(sources: &[&str], files: &[(&str, &str)]) -> Vec<(String, usize, usize)> {
        located(&citations(sources, files), sources[0])
    }

    fn scanned_text<'a>(files: &[(&'a str, &'a str)], citing: &str) -> &'a str {
        files
            .iter()
            .find(|(path, _)| *path == citing)
            .map(|(_, text)| *text)
            .unwrap_or_else(|| panic!("{citing} is not one of the scanned files"))
    }

    /// The scanned line a location points at.
    fn cited_line<'a>(files: &[(&'a str, &'a str)], location: &(String, usize, usize)) -> &'a str {
        let (citing, line, _) = location;
        scanned_text(files, citing)
            .lines()
            .nth(line - 1)
            .unwrap_or_else(|| panic!("{citing} has no line {line}"))
    }

    /// Whether the scanned text really cites `cited` where `location` says.
    ///
    /// Reading the named line and skipping the reported column's worth of
    /// characters must land on the cited path, which is what makes a reported
    /// position a citation the reader can follow.
    fn reads_back(files: &[(&str, &str)], location: &(String, usize, usize), cited: &str) -> bool {
        let (_, _, column) = location;
        cited_line(files, location)
            .chars()
            .skip(column - 1)
            .collect::<String>()
            .starts_with(cited)
    }

    /// Every occurrence of `cited` in `text`, whether or not it stands alone.
    ///
    /// The count comes from a candidate start at every character of the text,
    /// which shares no search with the scan under test: the scan skips ahead
    /// through the text as it matches, while this examines each position on its
    /// own and so cannot inherit a skipped position from it. It counts
    /// citations only for text whose every occurrence of `cited` names that
    /// path whole, which is how the fixtures above cite it.
    fn occurrences(text: &str, cited: &str) -> usize {
        text.char_indices()
            .filter(|(offset, _)| text[*offset..].starts_with(cited))
            .count()
    }

    /// The destination an archive of `source` publishes, so a fixture cites
    /// the path an operator repoints to instead of a copy of the layout rule.
    fn archived_destination(source: &str) -> String {
        let policy = ArtifactClassificationPolicy::configured(
            "dev",
            vec!["dev/active".into()],
            vec!["docs".into()],
            "dev/archive",
        );
        ArchiveLayout::new(
            &PlanTarget::Document {
                path: source.to_string(),
            },
            "dev/archive/abcdef12",
            &policy,
            &[],
        )
        .destination(source)
        .expect("a managed source has an archived destination")
    }

    #[test]
    fn test_citation_scanner_scan_records_one_location_per_occurrence_of_a_source() {
        let files = citation_fixture();
        let located = cited_located(&[CITED_SOURCE], &files);

        // Every location points at a real occurrence: reading the file it names
        // from the line and column it names yields the cited path.
        for location in &located {
            let (citing, line, column) = location;
            assert!(
                reads_back(&files, location, CITED_SOURCE),
                "{citing}:{line}:{column} does not locate a citation"
            );
        }
        // Every citation earns exactly one location, counted over the fixture
        // text independently of the scan, so a file citing the path twice on one
        // line is reported twice and a file citing another path is silent.
        for (citing, text) in &files {
            assert_eq!(
                located.iter().filter(|(cited, ..)| cited == citing).count(),
                occurrences(text, CITED_SOURCE),
                "{citing} earns one location per citation"
            );
        }
    }

    #[test]
    fn test_citation_scanner_scan_records_nothing_for_a_citation_repointed_to_the_archived_destination(
    ) {
        let destination = archived_destination(CITED_SOURCE);
        assert_ne!(destination, CITED_SOURCE);
        let text = format!("Superseded, see `{destination}` for the design.\n");
        let files = [("dev/studies/tooling.md", text.as_str())];

        assert!(
            cited_located(&[CITED_SOURCE], &files).is_empty(),
            "a citation naming the destination is already repointed"
        );
    }

    #[test]
    fn test_citation_scanner_scan_reports_a_citation_of_only_the_source_path_at_the_column_it_begins(
    ) {
        let line = format!("See `{CITED_SOURCE}` for the design.");
        let text = format!("{line}\n");
        let files = [("dev/studies/tooling.md", text.as_str())];

        let located = cited_located(&[CITED_SOURCE], &files);

        // The one citation is reported where the cited path begins, located
        // independently of the scan by examining each character position of the
        // line on its own.
        let begins = line
            .char_indices()
            .position(|(offset, _)| line[offset..].starts_with(CITED_SOURCE))
            .map(|index| index + 1)
            .expect("the fixture must cite the path for the column to mean anything");
        assert_eq!(
            located
                .iter()
                .map(|(_, _, column)| *column)
                .collect::<Vec<_>>(),
            vec![begins]
        );
        assert!(reads_back(&files, &located[0], CITED_SOURCE));
    }

    #[test]
    fn test_citation_scanner_scan_records_once_for_a_line_citing_one_source_and_another_sources_destination(
    ) {
        let repointed = "dev/active/design.md";
        let destination = archived_destination(repointed);
        let text = format!("Superseded `{CITED_SOURCE}`, now `{destination}`.\n");
        let files = [("dev/studies/tooling.md", text.as_str())];

        let index = citations(&[CITED_SOURCE, repointed], &files);

        let located = [CITED_SOURCE, repointed]
            .iter()
            .flat_map(|source| located(&index, source))
            .collect::<Vec<_>>();
        assert_eq!(located.len(), 1, "{text} cites one source path");
        assert!(
            reads_back(&files, &located[0], CITED_SOURCE),
            "the reported citation is the one naming a source path"
        );
    }

    #[test]
    fn test_citation_scanner_scan_records_each_markdown_link_resolving_to_a_source_once() {
        // Links that never spell the source whole, one per file, each recorded
        // where the link's destination is written.
        let linked = [
            ("dev/active/sibling.md", "See [the plan](plan.md#goal).\n"),
            (
                "dev/notes/nested.md",
                "See [the plan](../active/./plan.md).\n",
            ),
            (
                "dev/notes/angle.md",
                "See [the plan](<../active/plan.md>).\n",
            ),
            ("notes/rooted.md", "See [the plan](/dev/active/plan.md).\n"),
            (
                "dev/notes/reference.md",
                "See [the plan][p] and [again][p].\n\n[p]: ../active/plan.md\n",
            ),
        ];
        // A link the whole-path search already cites, and links resolving
        // elsewhere.
        let spelled = ("notes/both.md", "See [the plan](../dev/active/plan.md).\n");
        let elsewhere = (
            "notes/unresolved.md",
            "[up](../../escape.md) [web](https://example.com/plan.md) [here](#plan) [other](plan.md)\n",
        );
        let files = linked
            .iter()
            .copied()
            .chain([spelled, elsewhere])
            .collect::<Vec<_>>();

        let located = cited_located(&[CITED_SOURCE], &files);

        for (citing, _) in linked {
            let at = located
                .iter()
                .filter(|(cited, ..)| cited == citing)
                .collect::<Vec<_>>();
            assert_eq!(at.len(), 1, "{citing} cites the source once: {at:?}");
            let from = cited_line(&files, at[0])
                .chars()
                .skip(at[0].2 - 1)
                .collect::<String>();
            assert!(
                from.starts_with('['),
                "{citing} is located at its link: {from}"
            );
        }
        let both = located
            .iter()
            .filter(|(cited, ..)| cited == spelled.0)
            .collect::<Vec<_>>();
        assert_eq!(both.len(), 1, "a spelled link is one citation: {both:?}");
        assert!(reads_back(&files, both[0], CITED_SOURCE));
        assert!(located.iter().all(|(cited, ..)| cited != elsewhere.0));
    }

    /// Whether the one occurrence of the cited path in `line` cites it.
    fn cites(line: &str) -> bool {
        let offset = line
            .find(CITED_SOURCE)
            .expect("the fixture must carry the cited path");
        is_whole_path_citation(line, offset, CITED_SOURCE)
    }

    #[test]
    fn test_is_whole_path_citation_separates_the_cited_path_from_a_longer_path_around_it() {
        let canonical_destination = archived_destination(CITED_SOURCE);
        assert!(!canonical_destination.contains(CITED_SOURCE));
        // Text naming the path and nothing more. What sits against it carries
        // no name of its own: the delimiters prose and code write, a shell
        // default's `-`, a sentence period, a relative prefix that adds no
        // directory.
        for line in [
            format!("See `{CITED_SOURCE}` for the design."),
            format!("See [the design]({CITED_SOURCE})."),
            format!("OUT=\"${{PLAN_OUT:-{CITED_SOURCE}}}\""),
            format!("The design lives in {CITED_SOURCE}."),
            format!("See ./{CITED_SOURCE} from the repository root."),
            format!("See ../{CITED_SOURCE} from a nested directory."),
        ] {
            assert!(cites(&line), "{line} cites the path");
        }
        // Text whose path continues past the occurrence in either direction.
        for line in [
            format!("Vendored at `my{CITED_SOURCE}`."),
            format!("Rendered as `{CITED_SOURCE}x`."),
            format!("Superseded by `{CITED_SOURCE}-old`."),
            format!("Stored at `{CITED_SOURCE}.bak`."),
            format!("Stored at `other.{CITED_SOURCE}`."),
            format!("Stored at `.../{CITED_SOURCE}`."),
        ] {
            assert!(!cites(&line), "{line} names a longer path");
        }
    }

    #[test]
    fn test_is_whole_path_citation_recognizes_only_closed_standalone_shell_defaults() {
        for line in [
            format!("OUT=\"${{PLAN_OUT:-{CITED_SOURCE}}}\""),
            format!("${{_PLAN_OUT9:-{CITED_SOURCE}}}"),
            format!("OUT=\"${{PLAN_OUT:-./{CITED_SOURCE}}}\""),
        ] {
            assert!(cites(&line), "{line} is a shell default naming the path");
        }

        for line in [
            format!("notes:-{CITED_SOURCE}"),
            format!("notes:-./{CITED_SOURCE}"),
            format!("${{9PLAN_OUT:-{CITED_SOURCE}}}"),
            format!("${{PLAN-OUT:-{CITED_SOURCE}}}"),
            format!("${{PLAN_OUT:-{CITED_SOURCE}"),
            format!("prefix${{PLAN_OUT:-{CITED_SOURCE}}}"),
            format!("${{PLAN_OUT:-{CITED_SOURCE}}}suffix"),
            format!("${{PLAN_OUT:-notes:-{CITED_SOURCE}}}"),
        ] {
            assert!(
                !cites(&line),
                "{line} is not a standalone shell default citation"
            );
        }
    }

    #[test]
    fn test_citation_scanner_scan_ignores_shell_default_lookalikes_that_do_not_name_the_source() {
        let text = [
            format!("notes:-{CITED_SOURCE}"),
            format!("${{9PLAN_OUT:-{CITED_SOURCE}}}"),
            format!("${{PLAN_OUT:-{CITED_SOURCE}"),
            format!("prefix${{PLAN_OUT:-{CITED_SOURCE}}}"),
            format!("${{PLAN_OUT:-{CITED_SOURCE}}}suffix"),
        ]
        .join("\n");
        let files = [("scripts/archive.sh", text.as_str())];

        assert!(
            cited_located(&[CITED_SOURCE], &files).is_empty(),
            "raw, malformed, and concatenated shell-default lookalikes do not cite the source"
        );
    }

    #[test]
    fn test_is_whole_path_citation_rejects_a_source_inside_a_path_with_non_alphanumeric_names() {
        let line = "See `other+dev/active/design.md` for the alternate design.";
        let source = "dev/active/design.md";
        let offset = line
            .find(source)
            .expect("the longer path must contain the source path");

        assert!(
            !is_whole_path_citation(line, offset, source),
            "a plus belongs to the preceding filename, not citation syntax"
        );
    }

    proptest! {
        #[test]
        fn test_is_whole_path_citation_rejects_path_safe_characters_adjacent_to_the_source(
            before in any::<u8>().prop_map(char::from).prop_filter(
                "a non-delimiter filename character",
                |character| *character != '\0' && *character != '/' && !is_citation_delimiter(*character),
            ),
            after in any::<u8>().prop_map(char::from).prop_filter(
                "a non-delimiter filename character",
                |character| *character != '\0' && *character != '/' && !is_citation_delimiter(*character),
            ),
        ) {
            let prefixed = format!("`{before}{CITED_SOURCE}`");
            let suffixed = format!("`{CITED_SOURCE}{after}continued`");

            prop_assert!(!cites(&prefixed), "{before:?} extends the path before its source");
            prop_assert!(!cites(&suffixed), "{after:?} extends the path after its source");
        }

        #[test]
        fn test_is_whole_path_citation_accepts_syntax_delimiters(
            delimiter in any::<u8>().prop_map(char::from).prop_filter(
                "a citation delimiter",
                |character| is_citation_delimiter(*character),
            ),
        ) {
            let line = format!("{delimiter}{CITED_SOURCE}{delimiter}");

            prop_assert!(cites(&line), "{delimiter:?} delimits a standalone citation");
        }

        #[test]
        fn test_is_whole_path_citation_accepts_a_period_only_before_sentence_context(
            follower in any::<u8>().prop_map(char::from).prop_filter(
                "a sentence-period follower",
                |character| is_sentence_period_follower(*character),
            ),
        ) {
            let line = format!("{CITED_SOURCE}.{follower}");

            prop_assert!(cites(&line), "a period before {follower:?} ends the citation");
        }
    }

    #[test]
    fn test_citation_scanner_scan_records_nothing_for_a_longer_path_ending_with_the_cited_path() {
        // Every way a longer path can carry the cited path inside it: a root
        // above it, a root above it named outside ASCII, a directory whose name
        // merely ends with its first segment, a longer name past its end, and
        // text chaining the path into itself.
        let cases = [
            (
                CITED_SOURCE,
                format!("Mirrored at `docs/mirror/{CITED_SOURCE}`.\n"),
            ),
            (
                CITED_SOURCE,
                format!("Mirrored at `äänet/{CITED_SOURCE}`.\n"),
            ),
            (CITED_SOURCE, format!("Vendored at `my{CITED_SOURCE}`.\n")),
            (CITED_SOURCE, format!("Stored at `{CITED_SOURCE}.bak`.\n")),
            (CITED_SOURCE, format!("Stored at `other.{CITED_SOURCE}`.\n")),
            (CITED_SOURCE, format!("Rendered as `{CITED_SOURCE}x`.\n")),
            (
                "dev/active/dev",
                "`dev/active/dev/active/dev`\n".to_string(),
            ),
        ];

        for (source, text) in cases {
            assert!(
                text.contains(source),
                "{text} must carry {source} inside it for the case to bite"
            );
            let files = [("dev/studies/tooling.md", text.as_str())];
            assert!(
                cited_located(&[source], &files).is_empty(),
                "{text} names a longer path and does not cite {source}"
            );
        }
    }

    #[test]
    fn test_citation_scanner_scan_reports_a_character_column_for_a_citation_among_multibyte_text() {
        // A repository whose roots are not ASCII: the cited path and the prose
        // ahead of it are both multi-byte, so a column counted in bytes would
        // overshoot the one the text reads back at.
        let source = "äänet/käynnissä/ääni.md";
        let files = [(
            "äänet/tutkimus/muistio.md",
            "# Ääni\n\nÄänitys: `äänet/käynnissä/ääni.md` on päivitetty.\n",
        )];
        let located = cited_located(&[source], &files);

        assert_eq!(located.len(), occurrences(files[0].1, source));
        assert!(located
            .iter()
            .all(|location| reads_back(&files, location, source)));
    }

    #[test]
    fn test_citation_scanner_locates_each_source_as_a_scan_of_that_source_alone() {
        // Nested and prefix-sharing sources overlap in the text, which a
        // multi-pattern search must not let shadow one another.
        let sources = [
            CITED_SOURCE,
            "dev/active",
            "dev/active/plan.md.bak",
            "dev/design/auth-design.md",
        ];
        let files = citation_fixture()
            .into_iter()
            .chain([(
                "dev/notes.md",
                "See dev/active and `dev/active/plan.md.bak` beside dev/active/plan.md.\n",
            )])
            .collect::<Vec<_>>();
        let together = citations(&sources, &files);
        for source in sources {
            assert_eq!(
                located(&together, source),
                located(&citations(&[source], &files), source),
                "{source} is located the same with or without the other sources"
            );
        }
        assert!(
            sources
                .iter()
                .all(|source| together.citations(source).next().is_some()),
            "every fixture source is cited somewhere"
        );
    }

    #[test]
    fn test_citation_scanner_locates_citations_in_crlf_text_as_in_lf_text() {
        let lf = "intro\nsee dev/active/plan.md\n\n`dev/active/plan.md` and dev/active/plan.md";
        let crlf = lf.replace('\n', "\r\n");
        let lf_citations = citations(&[CITED_SOURCE], &[("notes.md", lf)]);
        assert_eq!(
            lf_citations.citations(CITED_SOURCE).count(),
            3,
            "the fixture cites three times"
        );
        assert_eq!(
            citations(&[CITED_SOURCE], &[("notes.md", &crlf)]),
            lf_citations
        );
    }

    #[test]
    fn test_citation_scanner_scan_reports_citations_in_code_comments_inline_code_spans_and_shell_scripts(
    ) {
        let files = citation_fixture();
        let located = cited_located(&[CITED_SOURCE], &files);
        let reported = |citing: &str| {
            located
                .iter()
                .filter(|(cited, ..)| cited == citing)
                .collect::<Vec<_>>()
        };

        // A production module doc comment, which no document edge records.
        let comment = reported("crates/jit/src/storage/claim_coordinator.rs");
        assert!(comment
            .iter()
            .all(|location| cited_line(&files, location).trim_start().starts_with("//!")));
        assert_eq!(comment.len(), 1);

        // An adopter-facing citation inside a markdown inline code span.
        let span = reported("docs/tutorials/parallel-work-worktrees.md");
        assert!(span.iter().all(|location| {
            let (_, _, column) = location;
            let line = cited_line(&files, location);
            let before = line.chars().take(column - 1).collect::<String>();
            let after = line.chars().skip(column - 1 + CITED_SOURCE.chars().count());
            before.ends_with('`') && after.clone().next() == Some('`')
        }));
        assert_eq!(span.len(), 1);

        // A shell script, in a comment and in an executable default alike.
        let script = reported("scripts/benchmark-rust-build.sh");
        let commented = script
            .iter()
            .filter(|location| cited_line(&files, location).trim_start().starts_with('#'))
            .count();
        assert_eq!(commented, 1);
        assert_eq!(script.len() - commented, 1);

        // A verified non-citation naming a different path stays silent.
        assert!(reported("docs/concepts/core-model.md").is_empty());
    }

    #[test]
    fn test_citation_location_wire_form_round_trips_through_its_citing_path_line_and_column() {
        let location = CitationLocation {
            citing: "notes/a:b.md".into(),
            line: 3,
            column: 7,
        };
        let wire = serde_json::to_value(&location).unwrap();

        assert_eq!(wire, "notes/a:b.md:3:7");
        assert_eq!(
            serde_json::from_value::<CitationLocation>(wire).unwrap(),
            location
        );
        assert!("notes/a.md:3".parse::<CitationLocation>().is_err());
    }

    #[test]
    fn test_citation_index_advise_reports_only_relocating_artifacts_once_across_plans() {
        let files = [(
            "notes/cite.md",
            "`dev/active/plan.md` `dev/active/kept.md`\n",
        )];
        let index = citations(&[CITED_SOURCE, "dev/active/kept.md"], &files);
        let plan = |entries: Vec<ArtifactPlanEntry>| {
            ArtifactPlan::new(
                PlanTarget::Document {
                    path: CITED_SOURCE.into(),
                },
                "dev/archive",
                PolicyStatus::Configured,
                entries,
                Vec::new(),
                Vec::new(),
            )
            .unwrap()
        };
        let moving = || {
            ArtifactPlanEntry::new(
                CITED_SOURCE,
                ArtifactVersion::WorkingTree,
                ArtifactAction::Move,
            )
            .with_destination(archived_destination(CITED_SOURCE))
            .with_content_identity(
                crate::domain::artifact_plan::ContentIdentity::from_bytes(b"plan"),
            )
        };
        let plans = [
            plan(vec![moving()]),
            plan(vec![
                moving(),
                ArtifactPlanEntry::new(
                    "dev/active/kept.md",
                    ArtifactVersion::WorkingTree,
                    ArtifactAction::Retain,
                ),
            ]),
        ];

        let advisory = index.advise(&plans);

        // The retained artifact is cited too, but its citation still resolves.
        assert!(index.citations("dev/active/kept.md").next().is_some());
        assert_eq!(
            advisory.citations,
            vec![ArtifactCitations {
                artifact: CITED_SOURCE.into(),
                locations: index.citations(CITED_SOURCE).cloned().collect(),
            }]
        );
    }
}
