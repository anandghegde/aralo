//! What an import did, snippet by snippet.
//!
//! The report is the product, not a log: the app shows it after an import and
//! the CLI prints it. The fidelity harness in `tests/fidelity.rs` measures
//! [`ImportReport::fidelity`] against a corpus, which is how M2's "90% or
//! better" exit criterion is checked rather than claimed.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::Format;

/// The outcome of one import run.
#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub format: Format,
    pub source: PathBuf,
    /// The library root the snippets went into.
    pub library: PathBuf,
    /// Nothing was written; the entries say what would have been.
    pub dry_run: bool,
    pub entries: Vec<Entry>,
    /// Trouble with the source as a whole rather than with one snippet.
    pub notes: Vec<Note>,
}

/// One snippet from the source.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub label: String,
    pub abbr: Vec<String>,
    pub group: Vec<String>,
    /// Where it was written, relative to the library root, or where it would
    /// be written on a dry run. `None` for anything skipped. It is written
    /// with `/` on every platform.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "slashed")]
    pub path: Option<PathBuf>,
    pub outcome: Outcome,
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Imported with nothing lost. These are the snippets fidelity counts.
    Clean,
    /// Imported, and it will expand, but something in it needs a human. The
    /// notes say what.
    NeedsEdit,
    /// Not imported.
    Skipped,
}

/// One thing worth telling the user about a snippet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    pub kind: NoteKind,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoteKind {
    /// A macro Aralo has no placeholder for. It is left in the body as
    /// literal text, so searching the library finds every one of them.
    Unconvertible,
    /// Converted, but not exactly. The snippet expands; the detail says what
    /// changed.
    Approximated,
    /// The file name was changed to keep it unique or usable.
    Renamed,
    /// Another snippet already claims this abbreviation.
    DuplicateAbbreviation,
    /// The source said something this importer could not read.
    Unreadable,
    /// No body and no abbreviation.
    Empty,
    /// The file could not be written: the detail is the file system's reason.
    NotWritten,
}

impl Note {
    pub fn new(kind: NoteKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    pub fn unconvertible(detail: impl Into<String>) -> Self {
        Self::new(NoteKind::Unconvertible, detail)
    }

    pub fn approximated(detail: impl Into<String>) -> Self {
        Self::new(NoteKind::Approximated, detail)
    }

    /// A note that leaves the snippet usable as it is.
    pub fn is_advisory(&self) -> bool {
        matches!(self.kind, NoteKind::Renamed)
    }
}

impl NoteKind {
    fn label(self) -> &'static str {
        match self {
            NoteKind::Unconvertible => "not converted",
            NoteKind::Approximated => "approximated",
            NoteKind::Renamed => "renamed",
            NoteKind::DuplicateAbbreviation => "duplicate abbreviation",
            NoteKind::Unreadable => "unreadable",
            NoteKind::Empty => "empty",
            NoteKind::NotWritten => "not written",
        }
    }
}

impl Entry {
    /// Builds an entry and derives its outcome from its notes: anything that
    /// is not purely advisory means a human has to look at it.
    pub fn new(label: String, abbr: Vec<String>, group: Vec<String>, notes: Vec<Note>) -> Self {
        let outcome = if notes.iter().all(Note::is_advisory) {
            Outcome::Clean
        } else {
            Outcome::NeedsEdit
        };
        Self {
            label,
            abbr,
            group,
            path: None,
            outcome,
            notes,
        }
    }

    /// An entry for something that was not imported at all.
    pub fn skipped(label: String, note: Note) -> Self {
        Self {
            label,
            abbr: Vec::new(),
            group: Vec::new(),
            path: None,
            outcome: Outcome::Skipped,
            notes: vec![note],
        }
    }
}

impl ImportReport {
    pub fn total(&self) -> usize {
        self.entries.len()
    }

    pub fn count(&self, outcome: Outcome) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.outcome == outcome)
            .count()
    }

    /// The snippets that were, or would have been, written.
    pub fn imported(&self) -> usize {
        self.total() - self.count(Outcome::Skipped)
    }

    /// The share of the source that converts with no manual edit, from 0 to 1.
    ///
    /// An empty source is 1.0: nothing failed to convert. The harness reports
    /// this as a percentage; `docs/format/import.md` defines what counts.
    pub fn fidelity(&self) -> f64 {
        if self.entries.is_empty() {
            return 1.0;
        }
        self.count(Outcome::Clean) as f64 / self.entries.len() as f64
    }

    /// Every note in the run, most common kind first, with how often it
    /// appeared. This is what the import report view lists at the top.
    pub fn summary(&self) -> Vec<(NoteKind, String, usize)> {
        let mut counts: std::collections::BTreeMap<(NoteKind, &str), usize> = Default::default();
        let all = self
            .notes
            .iter()
            .chain(self.entries.iter().flat_map(|entry| entry.notes.iter()));
        for note in all {
            *counts.entry((note.kind, note.detail.as_str())).or_default() += 1;
        }
        let mut rows: Vec<_> = counts
            .into_iter()
            .map(|((kind, detail), count)| (kind, detail.to_owned(), count))
            .collect();
        rows.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        rows
    }
}

impl fmt::Display for ImportReport {
    /// The text the CLI prints.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} {} from {}",
            if self.dry_run {
                "Would import"
            } else {
                "Imported"
            },
            plural(self.imported(), "snippet"),
            display(&self.source),
        )?;
        writeln!(
            f,
            "  format {}, into {}",
            self.format,
            display(&self.library)
        )?;
        writeln!(
            f,
            "  {} clean, {} need an edit, {} skipped ({:.0}% fidelity)",
            self.count(Outcome::Clean),
            self.count(Outcome::NeedsEdit),
            self.count(Outcome::Skipped),
            self.fidelity() * 100.0,
        )?;
        for (kind, detail, count) in self.summary() {
            writeln!(f, "  {} x{}: {}", kind.label(), count, detail)?;
        }
        Ok(())
    }
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn display(path: &Path) -> std::path::Display<'_> {
    path.display()
}

fn slashed<S: serde::Serializer>(path: &Option<PathBuf>, serializer: S) -> Result<S::Ok, S::Error> {
    match path {
        Some(path) => serializer.serialize_some(&aralo_library::slashed(path)),
        None => serializer.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(notes: Vec<Note>) -> Entry {
        Entry::new("x".into(), vec![";x".into()], Vec::new(), notes)
    }

    #[test]
    fn an_advisory_note_alone_still_counts_as_clean() {
        let entry = entry(vec![Note::new(NoteKind::Renamed, "thanks-2.md")]);
        assert_eq!(entry.outcome, Outcome::Clean);
    }

    #[test]
    fn an_unconvertible_macro_means_a_human_has_to_look() {
        let entry = entry(vec![Note::unconvertible("%delay:500%")]);
        assert_eq!(entry.outcome, Outcome::NeedsEdit);
    }

    #[test]
    fn fidelity_is_the_share_that_needs_no_edit() {
        let report = ImportReport {
            format: Format::Csv,
            source: PathBuf::from("a.csv"),
            library: PathBuf::from("lib"),
            dry_run: false,
            entries: vec![
                entry(Vec::new()),
                entry(Vec::new()),
                entry(Vec::new()),
                entry(vec![Note::unconvertible("%key:left%")]),
            ],
            notes: Vec::new(),
        };
        assert_eq!(report.fidelity(), 0.75);
        assert_eq!(report.imported(), 4);
    }

    #[test]
    fn an_empty_source_has_nothing_that_failed() {
        let report = ImportReport {
            format: Format::Csv,
            source: PathBuf::new(),
            library: PathBuf::new(),
            dry_run: false,
            entries: Vec::new(),
            notes: Vec::new(),
        };
        assert_eq!(report.fidelity(), 1.0);
    }

    #[test]
    fn the_summary_counts_the_same_note_once_with_a_total() {
        let report = ImportReport {
            format: Format::TextExpander,
            source: PathBuf::new(),
            library: PathBuf::new(),
            dry_run: false,
            entries: vec![
                entry(vec![Note::unconvertible("%delay:n%")]),
                entry(vec![Note::unconvertible("%delay:n%")]),
                entry(vec![Note::approximated("%key:tab%")]),
            ],
            notes: Vec::new(),
        };
        let summary = report.summary();
        assert_eq!(summary[0], (NoteKind::Unconvertible, "%delay:n%".into(), 2));
        assert_eq!(summary[1], (NoteKind::Approximated, "%key:tab%".into(), 1));
    }
}
