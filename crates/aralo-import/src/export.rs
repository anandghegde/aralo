//! A library out to JSON, YAML or CSV (M2 task 2.9).
//!
//! The output is the same [`SnippetRecord`] list an import reads, so
//! `export` then `import` is a round trip, and `tests/round_trip.rs` checks
//! that it loses nothing.

use aralo_library::Library;

use crate::document::{self, Document};
use crate::record::SnippetRecord;
use crate::{table, Format, ImportError};

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub format: Format,
    /// Export only this group and the groups inside it. Empty is the whole
    /// library.
    pub group: Vec<String>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: Format::Json,
            group: Vec::new(),
        }
    }
}

/// The bytes to write to a file. Snippets come out in path order, so two
/// exports of an unchanged library are the same file.
pub fn export(library: &Library, options: &ExportOptions) -> Result<Vec<u8>, ImportError> {
    if !options.format.is_writable() {
        return Err(ImportError::NotWritable(options.format));
    }
    let mut snippets: Vec<&aralo_library::LoadedSnippet> = library
        .snippets()
        .iter()
        .filter(|snippet| snippet.group.starts_with(&options.group))
        .collect();
    snippets.sort_by(|a, b| a.path.cmp(&b.path));

    let records: Vec<SnippetRecord> = snippets
        .iter()
        .map(|snippet| SnippetRecord::from_loaded(snippet))
        .collect();
    let name = library
        .manifest()
        .and_then(|manifest| manifest.name.clone());
    let document = Document::new(name, records);

    match options.format {
        Format::Json => document::write_json(&document),
        Format::Yaml => document::write_yaml(&document),
        Format::Csv => table::write(&document),
        Format::TextExpander => Err(ImportError::NotWritable(options.format)),
    }
}
