//! The CSV interchange: Aralo's own export and whatever else a spreadsheet
//! produced.
//!
//! Aralo writes every column it knows. Reading is looser: column names are
//! matched case-insensitively against a few common spellings, and a file with
//! no header at all is read positionally as abbreviation, body, label, which
//! is what TextExpander's CSV export looks like.
//!
//! Abbreviations and tags hold more than one value in one cell, separated by
//! newlines inside the quoted cell, so export then import loses nothing.

use aralo_snippet::{CaseMode, SnippetKind, TriggerMode};

use crate::document::Document;
use crate::record::SnippetRecord;
use crate::report::{Note, NoteKind};
use crate::{Candidate, ImportError, MacroPolicy, Parsed};

/// The columns Aralo writes, in order.
pub const COLUMNS: [&str; 12] = [
    "abbr",
    "label",
    "group",
    "tags",
    "body",
    "type",
    "trigger",
    "case",
    "word",
    "keep_delimiter",
    "enabled",
    "id",
];

/// A column Aralo writes that no other tool does. Seeing it in a header means
/// the file came from Aralo, so its bodies are already templates and must not
/// be run through the macro converter a second time.
const ARALO_MARKER: &str = "keep_delimiter";

/// Which of our fields a header cell names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Abbr,
    Label,
    Group,
    Tags,
    Body,
    Kind,
    Trigger,
    Case,
    Word,
    KeepDelimiter,
    Enabled,
    Id,
}

fn column_for(header: &str) -> Option<Column> {
    let header = header.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    Some(match header.as_str() {
        "abbr" | "abbreviation" | "abbreviations" | "shortcut" | "keyword" => Column::Abbr,
        "label" | "name" | "title" => Column::Label,
        "group" | "folder" | "category" | "collection" => Column::Group,
        "tags" | "tag" => Column::Tags,
        "body" | "content" | "text" | "snippet" | "expansion" | "replacement" => Column::Body,
        "type" | "kind" => Column::Kind,
        "trigger" => Column::Trigger,
        "case" => Column::Case,
        "word" | "whole_word" => Column::Word,
        "keep_delimiter" => Column::KeepDelimiter,
        "enabled" => Column::Enabled,
        "id" | "uuid" => Column::Id,
        _ => return None,
    })
}

pub fn read(bytes: &[u8], policy: MacroPolicy) -> Result<Parsed, ImportError> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .has_headers(false)
        .from_reader(bytes);
    let mut rows = reader.records();

    let Some(first) = rows.next() else {
        return Ok(Parsed::default());
    };
    let first = first.map_err(|source| ImportError::Source(source.to_string()))?;
    let header: Vec<Option<Column>> = first.iter().map(column_for).collect();

    let mut parsed = Parsed::default();
    // A file whose first row names no column we know has no header: read it
    // positionally, and read that first row as a snippet too.
    let (columns, from_aralo) = if header.iter().all(Option::is_none) {
        parsed.notes.push(Note::new(
            NoteKind::Approximated,
            "no header row: columns read as abbreviation, body, label",
        ));
        let positional = vec![Some(Column::Abbr), Some(Column::Body), Some(Column::Label)];
        row(&first, &positional, policy, &mut parsed);
        (positional, false)
    } else {
        let from_aralo = first
            .iter()
            .any(|cell| cell.trim().eq_ignore_ascii_case(ARALO_MARKER));
        (header, from_aralo)
    };

    let policy = if from_aralo {
        MacroPolicy::Template
    } else {
        policy
    };
    for record in rows {
        let record = record.map_err(|source| ImportError::Source(source.to_string()))?;
        row(&record, &columns, policy, &mut parsed);
    }
    Ok(parsed)
}

fn row(
    record: &csv::StringRecord,
    columns: &[Option<Column>],
    policy: MacroPolicy,
    parsed: &mut Parsed,
) {
    let mut snippet = SnippetRecord::default();
    let mut notes = Vec::new();
    let mut unreadable = Vec::new();

    for (cell, column) in record.iter().zip(columns) {
        let Some(column) = column else { continue };
        match column {
            Column::Abbr => snippet.abbr = split_lines(cell),
            Column::Label => snippet.label = cell.trim().to_owned(),
            Column::Group => {
                snippet.group = cell
                    .split(['/', '\n'])
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            Column::Tags => {
                snippet.tags = cell
                    .split(['\n', ',', ';'])
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            Column::Body => snippet.body = cell.to_owned(),
            Column::Kind => match cell.trim() {
                "" => {}
                "text" => snippet.kind = SnippetKind::Text,
                "rich" => snippet.kind = SnippetKind::Rich,
                "command" => snippet.kind = SnippetKind::Command,
                "prompt" => snippet.kind = SnippetKind::Prompt,
                "script" => snippet.kind = SnippetKind::Script,
                other => unreadable.push(format!("type: {other}")),
            },
            Column::Trigger => match cell.trim() {
                "" => {}
                "immediate" => snippet.trigger = Some(TriggerMode::Immediate),
                "delimiter" => snippet.trigger = Some(TriggerMode::Delimiter),
                other => unreadable.push(format!("trigger: {other}")),
            },
            Column::Case => match cell.trim() {
                "" => {}
                "exact" => snippet.case = Some(CaseMode::Exact),
                "ignore" => snippet.case = Some(CaseMode::Ignore),
                "adaptive" => snippet.case = Some(CaseMode::Adaptive),
                other => unreadable.push(format!("case: {other}")),
            },
            Column::Word => flag(cell, &mut snippet.word, "word", &mut unreadable),
            Column::KeepDelimiter => flag(
                cell,
                &mut snippet.keep_delimiter,
                "keep_delimiter",
                &mut unreadable,
            ),
            Column::Enabled => flag(cell, &mut snippet.enabled, "enabled", &mut unreadable),
            Column::Id => {
                if !cell.trim().is_empty() {
                    match cell.trim().parse() {
                        Ok(id) => snippet.id = Some(id),
                        Err(_) => unreadable.push(format!("id: {}", cell.trim())),
                    }
                }
            }
        }
    }

    for detail in unreadable {
        notes.push(Note::new(
            NoteKind::Unreadable,
            format!("a column value was not understood ({detail})"),
        ));
    }
    let (body, body_notes) = policy.apply(&snippet.body);
    snippet.body = body;
    notes.extend(body_notes);
    parsed.candidates.push(Candidate::new(snippet, notes));
}

fn flag(cell: &str, target: &mut Option<bool>, name: &str, unreadable: &mut Vec<String>) {
    match cell.trim().to_ascii_lowercase().as_str() {
        "" => {}
        "true" | "yes" | "1" | "y" => *target = Some(true),
        "false" | "no" | "0" | "n" => *target = Some(false),
        other => unreadable.push(format!("{name}: {other}")),
    }
}

fn split_lines(cell: &str) -> Vec<String> {
    cell.split('\n')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The CSV Aralo exports: every column, header first.
pub fn write(document: &Document) -> Result<Vec<u8>, ImportError> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    let error = |source: csv::Error| ImportError::Serialise(source.to_string());
    writer.write_record(COLUMNS).map_err(error)?;
    for snippet in &document.snippets {
        writer
            .write_record([
                snippet.abbr.join("\n"),
                snippet.label.clone(),
                snippet.group.join("/"),
                snippet.tags.join("\n"),
                snippet.body.clone(),
                kind_name(snippet.kind).to_owned(),
                snippet
                    .trigger
                    .map(|trigger| match trigger {
                        TriggerMode::Immediate => "immediate",
                        TriggerMode::Delimiter => "delimiter",
                    })
                    .unwrap_or_default()
                    .to_owned(),
                snippet
                    .case
                    .map(|case| match case {
                        CaseMode::Exact => "exact",
                        CaseMode::Ignore => "ignore",
                        CaseMode::Adaptive => "adaptive",
                    })
                    .unwrap_or_default()
                    .to_owned(),
                flag_text(snippet.word),
                flag_text(snippet.keep_delimiter),
                flag_text(snippet.enabled),
                snippet.id.map(|id| id.to_string()).unwrap_or_default(),
            ])
            .map_err(error)?;
    }
    writer
        .into_inner()
        .map_err(|error| ImportError::Serialise(error.error().to_string()))
}

fn kind_name(kind: SnippetKind) -> &'static str {
    match kind {
        SnippetKind::Text => "text",
        SnippetKind::Rich => "rich",
        SnippetKind::Command => "command",
        SnippetKind::Prompt => "prompt",
        SnippetKind::Script => "script",
    }
}

fn flag_text(flag: Option<bool>) -> String {
    match flag {
        Some(true) => "true".into(),
        Some(false) => "false".into(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_names_the_columns_in_any_order() {
        let text = "Content,Abbreviation,Folder\nHello there,;h,Work/Email\n";
        let parsed = read(text.as_bytes(), MacroPolicy::Auto).unwrap();
        let record = &parsed.candidates[0].record;
        assert_eq!(record.abbr, [";h"]);
        assert_eq!(record.body, "Hello there");
        assert_eq!(record.group, ["Work", "Email"]);
    }

    #[test]
    fn a_file_with_no_header_is_read_positionally() {
        let text = ";br,\"Best regards,\",Sign off\n";
        let parsed = read(text.as_bytes(), MacroPolicy::Auto).unwrap();
        assert_eq!(parsed.candidates.len(), 1);
        let record = &parsed.candidates[0].record;
        assert_eq!(record.abbr, [";br"]);
        assert_eq!(record.body, "Best regards,");
        assert_eq!(record.label, "Sign off");
    }

    #[test]
    fn macros_convert_when_a_body_carries_them() {
        let text = "abbr,body\n;d,%m/%d/%Y\n";
        let parsed = read(text.as_bytes(), MacroPolicy::Auto).unwrap();
        assert_eq!(parsed.candidates[0].record.body, "{{date: %m/%d/%Y}}");
    }

    #[test]
    fn aralos_own_header_turns_conversion_off_for_every_row() {
        let text = "abbr,body,keep_delimiter\n;d,%m/%d/%Y,\n";
        let parsed = read(text.as_bytes(), MacroPolicy::Auto).unwrap();
        assert_eq!(parsed.candidates[0].record.body, "%m/%d/%Y");
    }

    #[test]
    fn braces_in_a_plain_csv_are_escaped_so_they_stay_literal() {
        let text = "abbr,body\n;j,\"{{ not a placeholder }}\"\n";
        let parsed = read(text.as_bytes(), MacroPolicy::Auto).unwrap();
        assert_eq!(
            parsed.candidates[0].record.body,
            "\\{{ not a placeholder }}"
        );
    }

    #[test]
    fn an_unreadable_column_value_is_reported_and_the_row_still_imports() {
        let text = "abbr,body,case\n;x,hello,SHOUTY\n";
        let parsed = read(text.as_bytes(), MacroPolicy::Auto).unwrap();
        let candidate = &parsed.candidates[0];
        assert_eq!(candidate.record.body, "hello");
        assert_eq!(candidate.notes[0].kind, NoteKind::Unreadable);
    }

    #[test]
    fn more_than_one_abbreviation_fits_in_one_cell() {
        let text = "abbr,body\n\";br\n;rgds\",Best regards\n";
        let parsed = read(text.as_bytes(), MacroPolicy::Auto).unwrap();
        assert_eq!(parsed.candidates[0].record.abbr, [";br", ";rgds"]);
    }
}
