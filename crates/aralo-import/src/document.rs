//! The JSON and YAML interchange document.
//!
//! One shape for both, described by `schemas/export.schema.json`. This is what
//! `aralo export` writes and what `aralo import` reads back without losing
//! anything, which is M2's task 2.9.

use serde::{Deserialize, Serialize};

use crate::record::SnippetRecord;
use crate::report::{Note, NoteKind};
use crate::{Candidate, ImportError, MacroPolicy, Parsed};

/// The interchange document version. It follows the library format version,
/// so a file written by a newer Aralo is refused rather than half read.
pub const DOCUMENT_VERSION: u32 = aralo_snippet::FORMAT_VERSION;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub format: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub snippets: Vec<SnippetRecord>,
}

impl Document {
    pub fn new(name: Option<String>, snippets: Vec<SnippetRecord>) -> Self {
        Self {
            format: DOCUMENT_VERSION,
            name,
            snippets,
        }
    }

    fn check_version(&self) -> Result<(), ImportError> {
        if self.format > DOCUMENT_VERSION {
            return Err(ImportError::Source(format!(
                "this file is format {}, and this build reads {DOCUMENT_VERSION}",
                self.format
            )));
        }
        Ok(())
    }
}

/// Reads Aralo's JSON, and also a bare array of snippets, which is what a
/// hand-written file or another tool's export usually is.
pub fn read_json(text: &str, policy: MacroPolicy) -> Result<Parsed, ImportError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|source| ImportError::Source(source.to_string()))?;
    if value.is_array() {
        let snippets: Vec<SnippetRecord> = serde_json::from_value(value)
            .map_err(|source| ImportError::Source(source.to_string()))?;
        return Ok(loose(snippets, policy));
    }
    let document: Document =
        serde_json::from_value(value).map_err(|source| ImportError::Source(source.to_string()))?;
    document.check_version()?;
    Ok(own(document))
}

pub fn read_yaml(text: &str, policy: MacroPolicy) -> Result<Parsed, ImportError> {
    let value: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(text).map_err(|source| ImportError::Source(source.to_string()))?;
    if value.is_sequence() {
        let snippets: Vec<SnippetRecord> = serde_yaml_ng::from_value(value)
            .map_err(|source| ImportError::Source(source.to_string()))?;
        return Ok(loose(snippets, policy));
    }
    let document: Document = serde_yaml_ng::from_value(value)
        .map_err(|source| ImportError::Source(source.to_string()))?;
    document.check_version()?;
    Ok(own(document))
}

pub fn write_json(document: &Document) -> Result<Vec<u8>, ImportError> {
    let mut text = serde_json::to_vec_pretty(document)
        .map_err(|source| ImportError::Serialise(source.to_string()))?;
    text.push(b'\n');
    Ok(text)
}

pub fn write_yaml(document: &Document) -> Result<Vec<u8>, ImportError> {
    serde_yaml_ng::to_string(document)
        .map(String::into_bytes)
        .map_err(|source| ImportError::Serialise(source.to_string()))
}

/// Aralo's own document: the bodies are templates already.
fn own(document: Document) -> Parsed {
    let mut parsed = Parsed::default();
    for record in document.snippets {
        parsed.candidates.push(Candidate::new(record, Vec::new()));
    }
    parsed
}

/// A bare array from somewhere else: the bodies go through the policy.
fn loose(snippets: Vec<SnippetRecord>, policy: MacroPolicy) -> Parsed {
    let mut parsed = Parsed::default();
    parsed.notes.push(Note::new(
        NoteKind::Approximated,
        "a list of snippets with no format header: read as version 0",
    ));
    for mut record in snippets {
        let (body, notes) = policy.apply(&record.body);
        record.body = body;
        parsed.candidates.push(Candidate::new(record, notes));
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_round_trips_through_json_and_yaml() {
        let mut record = SnippetRecord::new("Hi {{cursor}}");
        record.abbr = vec![";h".into()];
        record.group = vec!["Work".into()];
        let document = Document::new(Some("Mine".into()), vec![record]);

        let json = String::from_utf8(write_json(&document).unwrap()).unwrap();
        let back = read_json(&json, MacroPolicy::Auto).unwrap();
        assert_eq!(back.candidates[0].record, document.snippets[0]);

        let yaml = String::from_utf8(write_yaml(&document).unwrap()).unwrap();
        let back = read_yaml(&yaml, MacroPolicy::Auto).unwrap();
        assert_eq!(back.candidates[0].record, document.snippets[0]);
    }

    #[test]
    fn a_newer_format_is_refused_rather_than_half_read() {
        let text = format!(r#"{{"format": {}, "snippets": []}}"#, DOCUMENT_VERSION + 1);
        assert!(read_json(&text, MacroPolicy::Auto).is_err());
    }

    #[test]
    fn a_bare_array_reads_and_its_macros_convert() {
        let text = r#"[{"abbr": ";d", "body": "%m/%d/%Y"}]"#;
        let parsed = read_json(text, MacroPolicy::Auto).unwrap();
        assert_eq!(parsed.candidates[0].record.body, "{{date: %m/%d/%Y}}");
        assert_eq!(parsed.notes.len(), 1);
    }

    #[test]
    fn aralos_own_document_is_never_converted_twice() {
        let text = r#"{"format": 0, "snippets": [{"body": "{{date: %m/%d/%Y}}"}]}"#;
        let parsed = read_json(text, MacroPolicy::Convert).unwrap();
        assert_eq!(parsed.candidates[0].record.body, "{{date: %m/%d/%Y}}");
    }
}
