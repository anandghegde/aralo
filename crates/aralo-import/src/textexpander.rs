//! TextExpander `.textexpander` property lists, XML or binary.
//!
//! The reader walks the property list by key rather than deserialising into a
//! struct, because the key set differs between TextExpander 3, 5 and 6 and an
//! export that carries one unknown key should still import.
//!
//! What is deliberately *not* mapped: the numeric per-snippet and per-group
//! option codes (`abbreviationMode`, `expandAfterMode`). Their meaning is not
//! documented anywhere this project can check, and guessing would silently
//! change how a snippet expands. Imported snippets inherit Aralo's defaults;
//! `docs/format/import.md` says so, and spike S7 is where a real export
//! settles it.

use plist::{Dictionary, Value};

use crate::record::SnippetRecord;
use crate::report::{Note, NoteKind};
use crate::{Candidate, ImportError, Parsed};

/// Keys that hold a group's snippets, oldest spelling last.
const SNIPPET_KEYS: [&str; 3] = ["snippetsTE2", "snippets", "snippetsTE3"];
/// Keys that hold a group's name.
const NAME_KEYS: [&str; 3] = ["groupName", "name", "label"];

/// Nesting deeper than this is not followed, matching the library loader.
const MAX_DEPTH: usize = 32;

pub fn read(bytes: &[u8]) -> Result<Parsed, ImportError> {
    let value = Value::from_reader(std::io::Cursor::new(bytes))
        .map_err(|source| ImportError::Source(source.to_string()))?;
    let mut parsed = Parsed::default();
    let mut found_a_group = false;
    walk(&value, &[], &mut parsed, &mut found_a_group, 0);
    if !found_a_group {
        return Err(ImportError::Source(
            "no snippets in the property list: expected a `snippetsTE2` or `snippets` array".into(),
        ));
    }
    Ok(parsed)
}

/// One group: its snippets, then the groups inside it, so the report reads in
/// the order the export is written.
fn walk(value: &Value, group: &[String], parsed: &mut Parsed, found: &mut bool, depth: usize) {
    if depth > MAX_DEPTH {
        parsed.notes.push(Note::new(
            NoteKind::Unreadable,
            "groups nested more than 32 deep were not read",
        ));
        return;
    }
    if let Some(array) = value.as_array() {
        *found = true;
        for snippet in array {
            read_snippet(snippet, group, parsed);
        }
        return;
    }
    let Some(dictionary) = value.as_dictionary() else {
        return;
    };
    let mut group = group.to_vec();
    if let Some(name) = first_string(dictionary, &NAME_KEYS) {
        group.push(name.to_owned());
    }
    if let Some(snippets) = SNIPPET_KEYS
        .iter()
        .find_map(|key| dictionary.get(key))
        .and_then(Value::as_array)
    {
        *found = true;
        for snippet in snippets {
            read_snippet(snippet, &group, parsed);
        }
    }
    if let Some(children) = dictionary.get("groups").and_then(Value::as_array) {
        for child in children {
            walk(child, &group, parsed, found, depth + 1);
        }
    }
}

fn read_snippet(value: &Value, group: &[String], parsed: &mut Parsed) {
    let Some(dictionary) = value.as_dictionary() else {
        parsed.candidates.push(Candidate::skipped(
            String::new(),
            Note::new(NoteKind::Unreadable, "an entry that is not a snippet"),
        ));
        return;
    };
    let label = first_string(dictionary, &["label"]).unwrap_or_default();
    let abbreviation = first_string(dictionary, &["abbreviation"]).unwrap_or_default();
    let plain = first_string(dictionary, &["plainText", "snippetText", "text"]);

    let Some(plain) = plain else {
        let name = if label.is_empty() {
            abbreviation
        } else {
            label
        };
        parsed.candidates.push(Candidate::skipped(
            name.to_owned(),
            Note::new(
                NoteKind::Unreadable,
                "a snippet with no plain text, such as a picture",
            ),
        ));
        return;
    };

    let (body, mut notes) = crate::macros::convert(plain);
    let kind = match dictionary
        .get("snippetType")
        .and_then(Value::as_signed_integer)
    {
        // 0 is plain text; anything unlabelled is treated as text too, since
        // the body already reads as text.
        None | Some(0) => aralo_snippet::SnippetKind::Text,
        Some(1) => {
            notes.push(Note::approximated(
                "formatting was dropped; the plain text was imported",
            ));
            aralo_snippet::SnippetKind::Text
        }
        Some(_) => {
            notes.push(Note::unconvertible(
                "a script snippet; Aralo does not run scripts yet",
            ));
            aralo_snippet::SnippetKind::Script
        }
    };

    let mut record = SnippetRecord::new(body);
    record.label = label.to_owned();
    record.group = group.to_vec();
    record.kind = kind;
    if !abbreviation.is_empty() {
        record.abbr = vec![abbreviation.to_owned()];
    }
    parsed.candidates.push(Candidate::new(record, notes));
}

fn first_string<'a>(dictionary: &'a Dictionary, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| dictionary.get(key).and_then(Value::as_string))
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_GROUP: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>groupName</key><string>Work</string>
  <key>snippetsTE2</key>
  <array>
    <dict>
      <key>abbreviation</key><string>;br</string>
      <key>label</key><string>Best regards</string>
      <key>plainText</key><string>Best regards,
%filltext:name=Name%</string>
      <key>snippetType</key><integer>0</integer>
    </dict>
    <dict>
      <key>abbreviation</key><string>;today</string>
      <key>plainText</key><string>%m/%d/%Y</string>
    </dict>
  </array>
</dict>
</plist>"#;

    #[test]
    fn a_group_becomes_a_folder_and_macros_convert() {
        let parsed = read(ONE_GROUP.as_bytes()).unwrap();
        assert_eq!(parsed.candidates.len(), 2);
        let first = &parsed.candidates[0].record;
        assert_eq!(first.group, ["Work"]);
        assert_eq!(first.abbr, [";br"]);
        assert_eq!(first.label, "Best regards");
        assert_eq!(first.body, "Best regards,\n{{field: Name}}");
        assert_eq!(parsed.candidates[1].record.body, "{{date: %m/%d/%Y}}");
    }

    #[test]
    fn nested_groups_keep_their_path() {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>groupName</key><string>Top</string>
  <key>groups</key>
  <array>
    <dict>
      <key>groupName</key><string>Inner</string>
      <key>snippetsTE2</key>
      <array><dict><key>abbreviation</key><string>;x</string><key>plainText</key><string>x</string></dict></array>
    </dict>
  </array>
  <key>snippetsTE2</key>
  <array><dict><key>abbreviation</key><string>;y</string><key>plainText</key><string>y</string></dict></array>
</dict>
</plist>"#;
        let parsed = read(text.as_bytes()).unwrap();
        let groups: Vec<_> = parsed
            .candidates
            .iter()
            .map(|candidate| candidate.record.group.join("/"))
            .collect();
        assert!(groups.contains(&"Top/Inner".to_string()));
        assert!(groups.contains(&"Top".to_string()));
    }

    #[test]
    fn a_script_is_kept_but_flagged() {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict><key>snippets</key><array><dict>
  <key>abbreviation</key><string>;sh</string>
  <key>plainText</key><string>date +%%s</string>
  <key>snippetType</key><integer>3</integer>
</dict></array></dict>
</plist>"#;
        let parsed = read(text.as_bytes()).unwrap();
        let candidate = &parsed.candidates[0];
        assert_eq!(candidate.record.kind, aralo_snippet::SnippetKind::Script);
        assert_eq!(candidate.record.body, "date +%s");
        assert_eq!(candidate.notes[0].kind, NoteKind::Unconvertible);
    }

    #[test]
    fn a_picture_snippet_is_skipped_not_invented() {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict><key>snippets</key><array><dict>
  <key>abbreviation</key><string>;logo</string>
  <key>snippetType</key><integer>1</integer>
</dict></array></dict>
</plist>"#;
        let parsed = read(text.as_bytes()).unwrap();
        assert!(parsed.candidates[0].is_skipped());
    }

    #[test]
    fn a_property_list_without_snippets_is_an_error() {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>other</key><string>x</string></dict></plist>"#;
        assert!(read(text.as_bytes()).is_err());
    }
}
