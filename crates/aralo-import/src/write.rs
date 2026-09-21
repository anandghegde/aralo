//! Records into a library folder: file names, folders and collisions.
//!
//! Naming is the part an import is judged on, because the file names are what
//! the user sees in Finder and in a diff. A snippet file is a lower-case
//! kebab-case slug of its label; a group keeps the name the source gave it,
//! minus anything a file system or the loader would object to.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use aralo_library::Library;
use aralo_snippet::SNIPPET_EXTENSION;

use crate::record::SnippetRecord;
use crate::report::{Note, NoteKind};

/// Longer file names are cut. Long enough to stay readable, short enough for
/// a deep folder on Windows.
const MAX_NAME: usize = 60;

/// Names Windows will not give a file, whatever the extension.
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Decides where each record goes and remembers what it has already used, so
/// two snippets called "Thanks" do not fight over one file.
#[derive(Debug)]
pub struct Placer {
    root: PathBuf,
    taken_paths: HashSet<String>,
    taken_abbr: HashSet<String>,
}

impl Placer {
    /// Seeded with what the library already holds, so an import never lands on
    /// an existing snippet.
    pub fn new(library: &Library) -> Self {
        let mut taken_paths = HashSet::new();
        let mut taken_abbr = HashSet::new();
        for snippet in library.snippets() {
            taken_paths.insert(key(&snippet.path));
            taken_abbr.extend(snippet.file.front.abbr.iter().cloned());
        }
        Self {
            root: library.root().to_owned(),
            taken_paths,
            taken_abbr,
        }
    }

    /// The path this record should be written to, relative to the library
    /// root, and whatever the user should know about how it was chosen.
    pub fn place(&mut self, record: &SnippetRecord, into: &[String]) -> (PathBuf, Vec<Note>) {
        let mut notes = Vec::new();
        let mut folder = PathBuf::new();
        for part in into.iter().chain(record.group.iter()) {
            let (name, changed) = folder_name(part);
            if changed {
                notes.push(Note::new(
                    NoteKind::Renamed,
                    format!("the group \"{part}\" became the folder \"{name}\""),
                ));
            }
            folder.push(name);
        }

        let stem = slug(record.display_name());
        let mut path = folder.join(format!("{stem}.{SNIPPET_EXTENSION}"));
        let mut suffix = 1;
        while self.taken_paths.contains(&key(&path)) || self.root.join(&path).exists() {
            suffix += 1;
            path = folder.join(format!("{stem}-{suffix}.{SNIPPET_EXTENSION}"));
        }
        if suffix > 1 {
            notes.push(Note::new(
                NoteKind::Renamed,
                format!("another snippet already used {stem}.{SNIPPET_EXTENSION}"),
            ));
        }
        self.taken_paths.insert(key(&path));

        for abbreviation in &record.abbr {
            if !self.taken_abbr.insert(abbreviation.clone()) {
                notes.push(Note::new(
                    NoteKind::DuplicateAbbreviation,
                    format!("{abbreviation} is already used; only one of them will expand"),
                ));
            }
        }
        (path, notes)
    }
}

/// A key that treats two names the same when the file system would. macOS and
/// Windows are case-insensitive, so `Thanks.md` and `thanks.md` collide.
fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase().replace('\\', "/")
}

/// A lower-case, kebab-case file name with no extension.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for character in name.chars() {
        if character.is_alphanumeric() {
            out.extend(character.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= MAX_NAME {
            break;
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() || RESERVED.contains(&out.as_str()) {
        return format!("snippet-{out}").trim_end_matches('-').to_owned();
    }
    out
}

/// A folder name the loader will read and every file system will take. The
/// group's own name is kept; only what would break is changed.
fn folder_name(name: &str) -> (String, bool) {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_control() || r#"<>:"/\|?*"#.contains(character) {
                '-'
            } else {
                character
            }
        })
        .collect();
    // The loader skips anything beginning with a dot or an underscore, and
    // Windows drops a trailing dot or space.
    let cleaned = cleaned
        .trim_start_matches(['.', '_'])
        .trim_end_matches(['.', ' '])
        .trim()
        .to_owned();
    let cleaned = if cleaned.is_empty() || RESERVED.contains(&cleaned.to_lowercase().as_str()) {
        format!("Group {cleaned}").trim_end().to_owned()
    } else {
        cleaned
    };
    let changed = cleaned != name;
    (cleaned, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_becomes_a_kebab_case_file_name() {
        assert_eq!(slug("Best regards"), "best-regards");
        assert_eq!(slug("Thanks!"), "thanks");
        assert_eq!(slug("  A/B  test "), "a-b-test");
        assert_eq!(slug("Grüße"), "grüße");
    }

    #[test]
    fn a_name_that_would_be_no_name_still_gets_one() {
        assert_eq!(slug(""), "snippet");
        assert_eq!(slug("!!!"), "snippet");
        assert_eq!(slug("con"), "snippet-con");
    }

    #[test]
    fn a_long_label_is_cut() {
        assert!(slug(&"word ".repeat(40)).len() <= MAX_NAME);
    }

    #[test]
    fn a_folder_keeps_its_name_unless_it_would_break() {
        assert_eq!(folder_name("Work"), ("Work".into(), false));
        assert_eq!(folder_name("_Archive"), ("Archive".into(), true));
        assert_eq!(folder_name("a/b"), ("a-b".into(), true));
        assert_eq!(folder_name("."), ("Group".into(), true));
    }
}
