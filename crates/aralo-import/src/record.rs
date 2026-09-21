//! The one snippet shape every importer produces and every exporter consumes.

use aralo_library::LoadedSnippet;
use aralo_snippet::{CaseMode, FrontMatter, SnippetFile, SnippetId, SnippetKind, TriggerMode};
use serde::{Deserialize, Serialize};

/// One snippet as the interchange formats carry it: a snippet file plus the
/// group path that says which folder it belongs in.
///
/// Importers fill this in; [`crate::write`] turns it into a file, and
/// [`crate::export`] turns a loaded library back into a list of these. Keeping
/// one definition is why export lives in this crate (ADR-0013).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SnippetRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<SnippetId>,
    #[serde(default)]
    pub label: String,
    /// Every abbreviation that expands this snippet. A single string is
    /// accepted on read.
    #[serde(default, deserialize_with = "one_or_many")]
    pub abbr: Vec<String>,
    /// Folder names from the library root down to the snippet's group.
    #[serde(default, deserialize_with = "group_path")]
    pub group: Vec<String>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub tags: Vec<String>,
    #[serde(rename = "type", default)]
    pub kind: SnippetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<TriggerMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case: Option<CaseMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_delimiter: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// The template text, with Aralo placeholders.
    #[serde(default)]
    pub body: String,
}

impl SnippetRecord {
    /// A record with a body and nothing else decided yet.
    pub fn new(body: impl Into<String>) -> Self {
        Self {
            body: body.into(),
            ..Self::default()
        }
    }

    /// The label a file name is built from: the label if there is one, else
    /// the first abbreviation, else the first line of the body.
    pub fn display_name(&self) -> &str {
        if !self.label.is_empty() {
            return &self.label;
        }
        if let Some(first) = self.abbr.first() {
            return first;
        }
        self.body.lines().next().unwrap_or_default()
    }

    /// Nothing worth importing: no body and no abbreviation.
    pub fn is_empty(&self) -> bool {
        self.body.trim().is_empty() && self.abbr.is_empty()
    }

    /// The snippet file this record becomes on disk.
    pub fn to_snippet_file(&self) -> SnippetFile {
        SnippetFile {
            front: FrontMatter {
                id: self.id,
                label: self.label.clone(),
                abbr: self.abbr.clone(),
                kind: self.kind,
                trigger: self.trigger,
                case: self.case,
                word: self.word,
                keep_delimiter: self.keep_delimiter,
                enabled: self.enabled,
                tags: self.tags.clone(),
                ai: None,
                extra: Default::default(),
            },
            body: self.body.clone(),
        }
    }

    /// The record for a snippet already in a library.
    pub fn from_loaded(snippet: &LoadedSnippet) -> Self {
        let front = &snippet.file.front;
        Self {
            // A snippet with no `id` in its file gets a temporary one at load;
            // exporting that would invent an identity, so it is left out.
            id: (!snippet.id_is_temporary).then_some(snippet.id),
            label: front.label.clone(),
            abbr: front.abbr.clone(),
            group: snippet.group.clone(),
            tags: front.tags.clone(),
            kind: front.kind,
            trigger: front.trigger,
            case: front.case,
            word: front.word,
            keep_delimiter: front.keep_delimiter,
            enabled: front.enabled,
            body: snippet.file.body.clone(),
        }
    }
}

/// `abbr: ";br"` and `abbr: [";br", ";rgds"]` both read.
fn one_or_many<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Option::<OneOrMany>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(OneOrMany::One(one)) if one.is_empty() => Vec::new(),
        Some(OneOrMany::One(one)) => vec![one],
        Some(OneOrMany::Many(many)) => many,
    })
}

/// A group reads as a list of folder names or as one `"Work/Email"` string,
/// because that is what a hand-written CSV column holds.
fn group_path<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let parts = one_or_many(deserializer)?;
    Ok(parts
        .iter()
        .flat_map(|part| part.split('/'))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_abbreviation_reads_as_a_list() {
        let record: SnippetRecord = serde_json::from_str(r#"{"abbr": ";br"}"#).unwrap();
        assert_eq!(record.abbr, [";br"]);
    }

    #[test]
    fn a_group_path_splits_on_slashes() {
        let record: SnippetRecord = serde_json::from_str(r#"{"group": "Work / Email"}"#).unwrap();
        assert_eq!(record.group, ["Work", "Email"]);
    }

    #[test]
    fn a_group_list_keeps_its_parts() {
        let record: SnippetRecord =
            serde_json::from_str(r#"{"group": ["Work", "Email"]}"#).unwrap();
        assert_eq!(record.group, ["Work", "Email"]);
    }

    #[test]
    fn the_display_name_falls_back_to_the_abbreviation_then_the_body() {
        let mut record = SnippetRecord::new("Best regards,\nSam");
        assert_eq!(record.display_name(), "Best regards,");
        record.abbr = vec![";br".into()];
        assert_eq!(record.display_name(), ";br");
        record.label = "Best regards".into();
        assert_eq!(record.display_name(), "Best regards");
    }
}
