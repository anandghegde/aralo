//! Importers, the import report, and the interchange formats Aralo exports.
//!
//! Import and export are one crate because they share one definition of a
//! snippet outside the library folder, [`SnippetRecord`]. Export writes it,
//! import reads it, and `export` then `import` is a round trip a test can
//! check (ADR-0013).
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use std::path::Path;
//! let library = aralo_library::Library::load(Path::new("~/Aralo"))?;
//! let report = aralo_import::import(
//!     Path::new("Work.textexpander"),
//!     &library,
//!     &aralo_import::ImportOptions::default(),
//! )?;
//! println!("{:.0}% converted with no edit", report.fidelity() * 100.0);
//! # Ok(())
//! # }
//! ```
//!
//! What each format keeps and what it cannot is in `docs/format/import.md`.
//! The percentage is measured by `tests/fidelity.rs` against the corpus in
//! `fixtures/import/`, which is how M2's exit criterion is checked.

mod document;
mod export;
mod macros;
mod record;
mod report;
mod table;
mod textexpander;
mod write;

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use aralo_library::Library;
use aralo_snippet::SnippetId;
use serde::Serialize;

pub use document::{Document, DOCUMENT_VERSION};
pub use export::{export, ExportOptions};
pub use record::SnippetRecord;
pub use report::{Entry, ImportReport, Note, NoteKind, Outcome};
pub use table::COLUMNS as CSV_COLUMNS;

/// A format Aralo reads. Only three of them are also written; see
/// [`Format::is_writable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// TextExpander's `.textexpander` property list, XML or binary.
    TextExpander,
    /// Aralo's CSV, and any other comma-separated file.
    Csv,
    Json,
    Yaml,
}

impl Format {
    pub const NAMES: [&'static str; 4] = ["textexpander", "csv", "json", "yaml"];

    /// Aralo exports the three interchange formats. It does not write
    /// TextExpander files: that would claim a fidelity in the other direction
    /// that nothing here measures.
    pub fn is_writable(self) -> bool {
        self != Format::TextExpander
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Format::TextExpander => "textexpander",
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Yaml => "yaml",
        })
    }
}

impl FromStr for Format {
    type Err = ImportError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_lowercase().as_str() {
            "textexpander" | "te" | "plist" => Ok(Format::TextExpander),
            "csv" | "tsv" => Ok(Format::Csv),
            "json" => Ok(Format::Json),
            "yaml" | "yml" => Ok(Format::Yaml),
            other => Err(ImportError::UnknownFormat(other.to_owned())),
        }
    }
}

/// What to do with the text in a body.
///
/// A reader that knows its source is Aralo's own, because the file says so,
/// uses [`MacroPolicy::Template`] whatever the caller asked for: nothing else
/// can tell a converted body from one that was always a template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MacroPolicy {
    /// Convert TextExpander macros when the body plainly carries them, and
    /// otherwise treat it as literal text. This is what an import does unless
    /// told otherwise.
    #[default]
    Auto,
    /// Convert, even where the macros are only a maybe. This turns `%d` in a
    /// code snippet into a date, so it is a choice, not a default.
    Convert,
    /// Never convert. The body is literal text, so a `{{` in it is escaped
    /// and stays literal.
    Literal,
    /// The body is an Aralo template already. Take it as it is.
    Template,
}

impl MacroPolicy {
    fn apply(self, body: &str) -> (String, Vec<Note>) {
        match self {
            MacroPolicy::Template => (body.to_owned(), Vec::new()),
            MacroPolicy::Convert => macros::convert(body),
            MacroPolicy::Literal => (macros::escape(body), Vec::new()),
            MacroPolicy::Auto if macros::looks_like_textexpander(body) => macros::convert(body),
            MacroPolicy::Auto => (macros::escape(body), Vec::new()),
        }
    }
}

impl FromStr for MacroPolicy {
    type Err = ImportError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_lowercase().as_str() {
            "auto" => Ok(MacroPolicy::Auto),
            "convert" => Ok(MacroPolicy::Convert),
            "literal" => Ok(MacroPolicy::Literal),
            "template" => Ok(MacroPolicy::Template),
            other => Err(ImportError::UnknownFormat(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// `None` asks [`detect`] to work it out from the file.
    pub format: Option<Format>,
    /// Put everything under this group, above whatever group the source gives.
    pub into: Vec<String>,
    pub macros: MacroPolicy,
    /// Work out what would happen and write nothing.
    pub dry_run: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("cannot read {path}: {source}")]
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot tell what format {0} is; say which with --format")]
    Undetectable(PathBuf),
    #[error("{0} is not a format Aralo knows; one of: textexpander, csv, json, yaml")]
    UnknownFormat(String),
    #[error("{0}")]
    Source(String),
    #[error("cannot write the export: {0}")]
    Serialise(String),
    #[error("Aralo does not write {0} files")]
    NotWritable(Format),
    #[error(transparent)]
    Library(#[from] aralo_library::LibraryError),
}

/// Works out a source's format from its first bytes, then its extension.
/// Content wins, because a property list saved as `.xml` is still a property
/// list.
pub fn detect(path: &Path, bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(b"bplist00") {
        return Some(Format::TextExpander);
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]);
    if head.contains("<plist") {
        return Some(Format::TextExpander);
    }
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
    match extension.as_deref() {
        Some("textexpander" | "plist") => return Some(Format::TextExpander),
        Some("json") => return Some(Format::Json),
        Some("yaml" | "yml") => return Some(Format::Yaml),
        Some("csv" | "tsv" | "txt") => return Some(Format::Csv),
        _ => {}
    }
    match head.trim_start().chars().next() {
        Some('{' | '[') => Some(Format::Json),
        _ => None,
    }
}

/// Reads `source` and writes its snippets into `library`.
///
/// The library is not reloaded; the caller does that, because it owns it. The
/// report says what happened to every snippet in the source, including the
/// ones that were left alone.
pub fn import(
    source: &Path,
    library: &Library,
    options: &ImportOptions,
) -> Result<ImportReport, ImportError> {
    let bytes = std::fs::read(source).map_err(|error| ImportError::Unreadable {
        path: source.to_owned(),
        source: error,
    })?;
    let format = options
        .format
        .or_else(|| detect(source, &bytes))
        .ok_or_else(|| ImportError::Undetectable(source.to_owned()))?;
    let mut report = import_bytes(&bytes, format, library, options)?;
    report.source = source.to_owned();
    Ok(report)
}

/// The same, for a source that is not a file: the app hands over what the user
/// dropped on the window.
pub fn import_bytes(
    bytes: &[u8],
    format: Format,
    library: &Library,
    options: &ImportOptions,
) -> Result<ImportReport, ImportError> {
    let text = || String::from_utf8_lossy(bytes).into_owned();
    let parsed = match format {
        Format::TextExpander => textexpander::read(bytes)?,
        Format::Csv => table::read(bytes, options.macros)?,
        Format::Json => document::read_json(&text(), options.macros)?,
        Format::Yaml => document::read_yaml(&text(), options.macros)?,
    };

    let mut placer = write::Placer::new(library);
    let mut report = ImportReport {
        format,
        source: PathBuf::new(),
        library: library.root().to_owned(),
        dry_run: options.dry_run,
        entries: Vec::with_capacity(parsed.candidates.len()),
        notes: parsed.notes,
    };

    for candidate in parsed.candidates {
        if let Some(note) = candidate.skip {
            report
                .entries
                .push(Entry::skipped(candidate.record.label, note));
            continue;
        }
        let mut record = candidate.record;
        if record.is_empty() {
            report.entries.push(Entry::skipped(
                record.display_name().to_owned(),
                Note::new(NoteKind::Empty, "no body and no abbreviation"),
            ));
            continue;
        }

        let (path, placing) = placer.place(&record, &options.into);
        let mut notes = candidate.notes;
        notes.extend(placing);
        // A snippet on disk always has an identity, so one is made here
        // rather than on the first save.
        record.id = record.id.or_else(|| Some(SnippetId::generate()));

        // The group a snippet ends up in is where the file goes, so it is the
        // one the caller asked for above the one the source gave.
        let group: Vec<String> = options
            .into
            .iter()
            .chain(record.group.iter())
            .cloned()
            .collect();
        let mut entry = Entry::new(
            record.display_name().to_owned(),
            record.abbr.clone(),
            group,
            notes,
        );
        entry.path = Some(path.clone());
        if !options.dry_run {
            if let Err(error) = library.write_snippet(&path, &record.to_snippet_file()) {
                report.entries.push(Entry::skipped(
                    entry.label,
                    Note::new(NoteKind::NotWritten, error.to_string()),
                ));
                continue;
            }
        }
        report.entries.push(entry);
    }
    Ok(report)
}

/// What a reader produced, before anything is written.
#[derive(Debug, Default)]
struct Parsed {
    candidates: Vec<Candidate>,
    /// Trouble with the source as a whole rather than with one snippet.
    notes: Vec<Note>,
}

#[derive(Debug)]
struct Candidate {
    record: SnippetRecord,
    notes: Vec<Note>,
    /// Set when the source entry cannot become a snippet at all.
    skip: Option<Note>,
}

impl Candidate {
    fn new(record: SnippetRecord, notes: Vec<Note>) -> Self {
        Self {
            record,
            notes,
            skip: None,
        }
    }

    fn skipped(label: String, note: Note) -> Self {
        Self {
            record: SnippetRecord {
                label,
                ..SnippetRecord::default()
            },
            notes: Vec::new(),
            skip: Some(note),
        }
    }

    #[cfg(test)]
    fn is_skipped(&self) -> bool {
        self.skip.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_format_name_round_trips() {
        for name in Format::NAMES {
            let format: Format = name.parse().unwrap();
            assert_eq!(format.to_string(), name);
        }
        assert!("sqlite".parse::<Format>().is_err());
    }

    #[test]
    fn content_beats_the_extension() {
        let plist = br#"<?xml version="1.0"?><plist version="1.0"><dict/></plist>"#;
        assert_eq!(
            detect(Path::new("export.csv"), plist),
            Some(Format::TextExpander)
        );
        assert_eq!(
            detect(Path::new("export.bin"), b"bplist00xx"),
            Some(Format::TextExpander)
        );
    }

    #[test]
    fn an_extension_decides_when_the_content_says_nothing() {
        assert_eq!(detect(Path::new("a.csv"), b"a,b\n"), Some(Format::Csv));
        assert_eq!(
            detect(Path::new("a.yml"), b"snippets: []"),
            Some(Format::Yaml)
        );
        assert_eq!(detect(Path::new("a"), b"[{}]"), Some(Format::Json));
        assert_eq!(detect(Path::new("a"), b"hello"), None);
    }

    #[test]
    fn auto_converts_a_macro_and_leaves_a_lookalike_alone() {
        assert_eq!(MacroPolicy::Auto.apply("%m/%d/%Y").0, "{{date: %m/%d/%Y}}");
        // One specifier on its own is more likely to be a format string.
        assert_eq!(
            MacroPolicy::Auto.apply(r#"printf("%d")"#).0,
            r#"printf("%d")"#
        );
        assert_eq!(
            MacroPolicy::Convert.apply(r#"printf("%d")"#).0,
            r#"printf("{{date: %d}}")"#
        );
        // Literal text keeps its braces by escaping them.
        assert_eq!(MacroPolicy::Auto.apply("{{x}}").0, "\\{{x}}");
        assert_eq!(MacroPolicy::Literal.apply("%Y {{x}}").0, "%Y \\{{x}}");
        assert_eq!(MacroPolicy::Template.apply("%Y {{x}}").0, "%Y {{x}}");
    }
}
