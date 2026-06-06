//! Document format adapter system
//!
//! This module provides a pluggable adapter system for handling multiple document formats
//! (Markdown, AsciiDoc, reStructuredText, MDX, etc.). Format-specific operations like
//! asset scanning and link rewriting are isolated in adapter implementations.

mod adapter;
mod assets;
mod link_validator;
pub mod parser;

pub use adapter::{AdapterRegistry, DocFormatAdapter, HtmlAdapter, MarkdownAdapter};
pub use assets::{Asset, AssetScanner, AssetType};
pub use link_validator::{InternalLink, LinkType, LinkValidationResult, LinkValidator};
pub use parser::{slugify_heading, ContentParser, MarkdownContentParser, ParsedContent, Section};

#[cfg(feature = "html")]
pub use parser::HtmlContentParser;
#[cfg(feature = "xml")]
pub use parser::XmlContentParser;
