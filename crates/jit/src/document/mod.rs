//! Document format adapter system
//!
//! This module provides a pluggable adapter system for handling multiple document formats
//! (Markdown, AsciiDoc, reStructuredText, MDX, etc.). Format-specific operations like
//! asset scanning and link rewriting are isolated in adapter implementations.

mod adapter;
mod assets;
mod link_validator;

pub use adapter::{AdapterRegistry, DocFormatAdapter, MarkdownAdapter};
pub use assets::{Asset, AssetScanner, AssetType};
pub use link_validator::{InternalLink, LinkType, LinkValidationResult, LinkValidator};
