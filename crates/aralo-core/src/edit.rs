//! Changing the library: create, save, move and delete, for snippets and for
//! groups.
//!
//! Every operation here writes files and then reads the folder again, because
//! the folder is the truth ([ADR-0006]) and nothing else may drift from it. A
//! save is one file rewritten and one reload, so what the editor shows next is
//! what a text editor would show, and what `git diff` shows is one snippet.
//!
//! Aralo never writes a key it did not mean to write. A [`Draft`] carries the
//! fields a person edits; [`Core::save_snippet`] folds it into the front matter
//! that is already on disk, so an `ai:` block, a key from a future version and
//! anything a hand-written file carries all survive a save untouched.
//!
//! [ADR-0006]: ../../../docs/adr/0006-files-as-source-of-truth.md

use std::path::{Path, PathBuf};

use aralo_library::LoadedSnippet;
use aralo_snippet::{
    CaseMode, FrontMatter, GroupFile, SnippetFile, SnippetId, SnippetKind, TriggerMode,
    SNIPPET_EXTENSION,
};

use crate::{Core, CoreError};

/// The longest file name Aralo makes for itself, in characters. Long enough to
/// recognise a snippet in a folder listing, short enough to leave room for the
/// paths a sync client wraps around it.
const MAX_STEM: usize = 48;

/// What a person edits: a snippet's fields, and nothing about how it is stored.
///
/// A `None` means "inherit from the group"; it is not the same as the value the
/// group happens to resolve to today, which is why these are options and not
/// the resolved [`aralo_library::Settings`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Draft {
    pub label: String,
    pub abbr: Vec<String>,
    pub body: String,
    pub tags: Vec<String>,
    pub kind: SnippetKind,
    pub trigger: Option<TriggerMode>,
    pub case: Option<CaseMode>,
    /// Expand only after a non-word character.
    pub word: Option<bool>,
    /// Re-insert the delimiter that triggered the expansion.
    pub keep_delimiter: Option<bool>,
    pub enabled: Option<bool>,
}

impl Draft {
    /// The draft that opens an editor on a snippet already in the library.
    pub fn of(snippet: &LoadedSnippet) -> Self {
        let front = &snippet.file.front;
        Self {
            label: front.label.clone(),
            abbr: front.abbr.clone(),
            body: snippet.file.body.clone(),
            tags: front.tags.clone(),
            kind: front.kind,
            trigger: front.trigger,
            case: front.case,
            word: front.word,
            keep_delimiter: front.keep_delimiter,
            enabled: front.enabled,
        }
    }

    /// Writes this draft over `front`, leaving every key it says nothing about
    /// exactly as it was.
    pub(crate) fn apply(&self, front: &mut FrontMatter) {
        front.label = self.label.clone();
        front.abbr = self.abbr.clone();
        front.tags = self.tags.clone();
        front.kind = self.kind;
        front.trigger = self.trigger;
        front.case = self.case;
        front.word = self.word;
        front.keep_delimiter = self.keep_delimiter;
        front.enabled = self.enabled;
    }
}

/// Something about a draft the editor should say before it is saved.
///
/// None of these stops a save: a half-written snippet is a normal thing to have
/// on screen, and refusing to write it would lose the user's work. They are
/// what the editor underlines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftIssue {
    /// Which abbreviation it is about, when it is about one.
    pub abbr: Option<String>,
    pub problem: Problem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// An abbreviation that is only whitespace can never be typed.
    Blank,
    /// Another snippet already answers to this, so one of the two never fires.
    /// Which one is not worth guessing at; the editor names the other.
    Taken {
        by: SnippetId,
        /// What to call the other one in the message.
        name: String,
        /// Its path from the library root.
        path: PathBuf,
    },
    /// The same abbreviation twice in one snippet.
    Repeated,
    /// This build cannot expand a snippet of this type yet.
    UnsupportedKind(SnippetKind),
}

impl Core {
    /// Writes a new snippet into the group at `group` and returns its ID.
    ///
    /// The file is named after the snippet, so the folder reads like the
    /// library: a label of "Best regards" becomes `best-regards.md`. A name
    /// already in use gets a number, never an overwrite.
    pub fn create_snippet(
        &mut self,
        group: &[String],
        draft: &Draft,
    ) -> Result<SnippetId, CoreError> {
        let folder = group_folder(group)?;
        let id = SnippetId::generate();
        let mut front = FrontMatter {
            id: Some(id),
            ..FrontMatter::default()
        };
        draft.apply(&mut front);
        let file = SnippetFile {
            front,
            body: draft.body.clone(),
        };
        let path = self.free_path(&folder, &stem_for(draft));
        self.library.create_group(&folder)?;
        self.library.write_snippet(&path, &file)?;
        self.reload()?;
        Ok(id)
    }

    /// Writes `draft` over the snippet `id`, in the file it is already in.
    ///
    /// Returns the snippet's ID afterwards, which differs from `id` only for a
    /// hand-written file that carried none: that one is identified by its path
    /// until Aralo saves it, and this is the save that gives it a real ULID.
    pub fn save_snippet(&mut self, id: SnippetId, draft: &Draft) -> Result<SnippetId, CoreError> {
        let snippet = self.snippet_or_error(id)?;
        let path = snippet.path.clone();
        let mut file = snippet.file.clone();
        let id = match file.front.id {
            Some(id) => id,
            None => *file.front.id.insert(SnippetId::generate()),
        };
        draft.apply(&mut file.front);
        file.body = draft.body.clone();
        self.library.write_snippet(&path, &file)?;
        self.reload()?;
        Ok(id)
    }

    /// Switches one snippet on or off without touching anything else in it.
    pub fn set_snippet_enabled(&mut self, id: SnippetId, enabled: bool) -> Result<(), CoreError> {
        let snippet = self.snippet_or_error(id)?;
        let path = snippet.path.clone();
        let mut file = snippet.file.clone();
        file.front.enabled = Some(enabled);
        self.library.write_snippet(&path, &file)?;
        self.reload()
    }

    /// Moves a snippet into another group, keeping its file name and its ID.
    pub fn move_snippet(&mut self, id: SnippetId, group: &[String]) -> Result<(), CoreError> {
        let snippet = self.snippet_or_error(id)?;
        let from = snippet.path.clone();
        let folder = group_folder(group)?;
        let name = from
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("snippet.{SNIPPET_EXTENSION}")));
        let to = self.free_path(&folder, &stem_of(&name));
        if to == from {
            return Ok(());
        }
        self.library.create_group(&folder)?;
        self.library.move_path(&from, &to)?;
        self.reload()
    }

    /// Removes a snippet's file.
    ///
    /// It is unlinked, not put in the trash. A shell that would rather the user
    /// could get it back should move the file itself and call
    /// [`Core::reload`]; [`Core::snippet_path`] says where it is.
    pub fn delete_snippet(&mut self, id: SnippetId) -> Result<(), CoreError> {
        let path = self.snippet_or_error(id)?.path.clone();
        self.library.remove_file(&path)?;
        self.reload()
    }

    /// Where a snippet's file is, as an absolute path.
    pub fn snippet_path(&self, id: SnippetId) -> Option<PathBuf> {
        let snippet = self.library.snippet(id)?;
        Some(self.library.root().join(&snippet.path))
    }

    /// Creates an empty group. A group is a folder, so this is one `mkdir`; the
    /// `_group.yaml` arrives when something in it is set.
    pub fn create_group(&mut self, group: &[String]) -> Result<(), CoreError> {
        self.library.create_group(&group_folder(group)?)?;
        self.reload()
    }

    /// Writes a group's `_group.yaml`, keeping every key it already carries
    /// that `change` does not touch.
    pub fn edit_group<F>(&mut self, group: &[String], change: F) -> Result<(), CoreError>
    where
        F: FnOnce(&mut GroupFile),
    {
        let folder = group_folder(group)?;
        let mut file = match self.library.group(group) {
            Some(loaded) => loaded.file.clone(),
            None if group.is_empty() => GroupFile::default(),
            None => return Err(CoreError::NoSuchGroup(folder.display().to_string())),
        };
        change(&mut file);
        self.library.create_group(&folder)?;
        self.library.write_group(&folder, &file)?;
        self.reload()
    }

    /// Switches a group on or off. Off is sticky: nothing inside it expands,
    /// whatever a group further down says.
    pub fn set_group_enabled(&mut self, group: &[String], enabled: bool) -> Result<(), CoreError> {
        self.edit_group(group, |file| file.enabled = Some(enabled))
    }

    /// Renames a group's folder, and with it the group. Returns its new path.
    ///
    /// The `name` key in `_group.yaml`, if there is one, is renamed too:
    /// leaving it behind would rename the folder and nothing the user sees.
    pub fn rename_group(&mut self, group: &[String], name: &str) -> Result<Vec<String>, CoreError> {
        let name = name.trim();
        check_name(name)?;
        if group.is_empty() {
            return Err(CoreError::InvalidName {
                name: name.to_owned(),
                reason: "the library root is not a group that can be renamed".to_owned(),
            });
        }
        let from = group_folder(group)?;
        let mut renamed = group.to_vec();
        *renamed.last_mut().expect("the group is not the root") = name.to_owned();
        let to = group_folder(&renamed)?;
        if to == from {
            return Ok(renamed);
        }
        let had_name = self
            .library
            .group(group)
            .is_some_and(|loaded| loaded.file.name.is_some());
        self.library.move_path(&from, &to)?;
        self.reload()?;
        if had_name {
            self.edit_group(&renamed, |file| file.name = Some(name.to_owned()))?;
        }
        Ok(renamed)
    }

    /// Moves a group, and everything in it, inside another group. Returns its
    /// new path.
    pub fn move_group(
        &mut self,
        group: &[String],
        into: &[String],
    ) -> Result<Vec<String>, CoreError> {
        if group.is_empty() {
            return Err(CoreError::InvalidName {
                name: String::new(),
                reason: "the library root is not a group that can be moved".to_owned(),
            });
        }
        if into.starts_with(group) {
            return Err(CoreError::InvalidName {
                name: group.join("/"),
                reason: "a group cannot be moved inside itself".to_owned(),
            });
        }
        let from = group_folder(group)?;
        let mut moved = into.to_vec();
        moved.push(group.last().expect("the group is not the root").clone());
        let to = group_folder(&moved)?;
        if to == from {
            return Ok(moved);
        }
        self.library.move_path(&from, &to)?;
        self.reload()?;
        Ok(moved)
    }

    /// Removes a group's folder and everything in it. Returns the paths that
    /// went, relative to the library root.
    ///
    /// This is the user's data. [`Core::group_contents`] says how much of it
    /// there is, so a shell can ask before calling this.
    pub fn delete_group(&mut self, group: &[String]) -> Result<Vec<PathBuf>, CoreError> {
        let folder = group_folder(group)?;
        if self.library.group(group).is_none() {
            return Err(CoreError::NoSuchGroup(folder.display().to_string()));
        }
        let removed = self.library.remove_group(&folder)?;
        self.reload()?;
        Ok(removed)
    }

    /// How many snippets a group holds, counting the groups inside it.
    pub fn group_contents(&self, group: &[String]) -> usize {
        self.library
            .snippets()
            .iter()
            .filter(|snippet| snippet.group.starts_with(group))
            .count()
    }

    /// What the editor should say about a draft before it is saved: blank and
    /// repeated abbreviations, and ones another snippet already answers to.
    ///
    /// `editing` is the snippet being edited, so that a save does not report
    /// the snippet's own abbreviations as taken by itself. Pass `None` for a
    /// snippet that does not exist yet.
    pub fn check_draft(&self, draft: &Draft, editing: Option<SnippetId>) -> Vec<DraftIssue> {
        let mut issues = Vec::new();
        if !draft.kind.is_supported() {
            issues.push(DraftIssue {
                abbr: None,
                problem: Problem::UnsupportedKind(draft.kind),
            });
        }
        for (position, abbreviation) in draft.abbr.iter().enumerate() {
            if abbreviation.trim().is_empty() {
                issues.push(DraftIssue {
                    abbr: Some(abbreviation.clone()),
                    problem: Problem::Blank,
                });
                continue;
            }
            if draft.abbr[..position].contains(abbreviation) {
                issues.push(DraftIssue {
                    abbr: Some(abbreviation.clone()),
                    problem: Problem::Repeated,
                });
                continue;
            }
            if let Some(other) = self.claimed_by(abbreviation, editing) {
                issues.push(DraftIssue {
                    abbr: Some(abbreviation.clone()),
                    problem: Problem::Taken {
                        by: other.id,
                        name: other.display_name().to_owned(),
                        path: other.path.clone(),
                    },
                });
            }
        }
        issues
    }

    /// An abbreviation for a snippet called `label` that nothing else answers
    /// to yet: the initials of its words, or its first letters when it is one
    /// word, with a number on the end if it has to be.
    pub fn suggest_abbreviation(&self, label: &str) -> String {
        let words: Vec<&str> = label
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        let base: String = match words.as_slice() {
            [] => String::new(),
            [one] => one.chars().take(3).collect(),
            many => many
                .iter()
                .take(4)
                .filter_map(|word| word.chars().next())
                .collect(),
        }
        .to_lowercase();
        if base.is_empty() {
            return String::new();
        }
        if self.claimed_by(&base, None).is_none() {
            return base;
        }
        (2..100)
            .map(|n| format!("{base}{n}"))
            .find(|candidate| self.claimed_by(candidate, None).is_none())
            .unwrap_or(base)
    }

    fn claimed_by(&self, abbreviation: &str, except: Option<SnippetId>) -> Option<&LoadedSnippet> {
        self.library.snippets().iter().find(|snippet| {
            Some(snippet.id) != except
                && snippet.settings.enabled
                && snippet.file.front.abbr.iter().any(|a| a == abbreviation)
        })
    }

    fn snippet_or_error(&self, id: SnippetId) -> Result<&LoadedSnippet, CoreError> {
        self.library
            .snippet(id)
            .ok_or_else(|| CoreError::NoSuchSnippet(id.to_string()))
    }

    /// `folder/stem.md`, or the first numbered name after it that is free. Only
    /// the file system is asked: a name may be taken by a file the loader never
    /// reads, and writing over it would still lose someone's work.
    fn free_path(&self, folder: &Path, stem: &str) -> PathBuf {
        let root = self.library.root();
        let candidate = |suffix: String| folder.join(format!("{stem}{suffix}.{SNIPPET_EXTENSION}"));
        let first = candidate(String::new());
        if !root.join(&first).exists() {
            return first;
        }
        (2..)
            .map(|n| candidate(format!("-{n}")))
            .find(|path| !root.join(path).exists())
            .expect("the range is unbounded")
    }
}

/// The folder a group lives in, checked one name at a time so that nothing a
/// person types in a rename field can reach outside the library.
fn group_folder(group: &[String]) -> Result<PathBuf, CoreError> {
    let mut folder = PathBuf::new();
    for name in group {
        check_name(name)?;
        folder.push(name);
    }
    Ok(folder)
}

fn check_name(name: &str) -> Result<(), CoreError> {
    let invalid = |reason: &str| CoreError::InvalidName {
        name: name.to_owned(),
        reason: reason.to_owned(),
    };
    if name.is_empty() {
        return Err(invalid("a group needs a name"));
    }
    if name == "." || name == ".." {
        return Err(invalid("that is a folder, not a name"));
    }
    if name.contains(['/', '\\']) || name.contains('\0') {
        return Err(invalid("a name cannot hold a path separator"));
    }
    if name.starts_with('.') || name.starts_with('_') {
        return Err(invalid(
            "Aralo does not read folders whose name starts with a dot or an underscore",
        ));
    }
    Ok(())
}

/// The file name for a new snippet: its label, else its first abbreviation,
/// else a word that is better than nothing.
fn stem_for(draft: &Draft) -> String {
    let from_label = slug(&draft.label);
    if !from_label.is_empty() {
        return from_label;
    }
    let from_abbr = draft
        .abbr
        .first()
        .map(|abbreviation| slug(abbreviation))
        .unwrap_or_default();
    if !from_abbr.is_empty() {
        return from_abbr;
    }
    "snippet".to_owned()
}

fn stem_of(name: &Path) -> String {
    name.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("snippet")
        .to_owned()
}

/// A file name from a person's words: lower case, one hyphen where anything
/// that is not a letter or a digit was, and short enough to read.
///
/// Letters outside ASCII are kept, because a library written in Greek should
/// not be a folder of `snippet-2.md`. What the file system then makes of the
/// name is its business; the snippet's identity is its ULID, not its path.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars().take(MAX_STEM * 4) {
        if character.is_alphanumeric() {
            out.extend(character.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.trim_end_matches('-').chars().count() >= MAX_STEM {
            break;
        }
    }
    out.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_name_is_the_label_a_folder_listing_can_read() {
        assert_eq!(slug("Best regards"), "best-regards");
        assert_eq!(slug("  Invoice — 30 days  "), "invoice-30-days");
        assert_eq!(slug(";br"), "br");
        assert_eq!(slug("Καλημέρα"), "καλημέρα");
        assert_eq!(slug("!!!"), "");
        assert_eq!(slug(&"a b".repeat(80)).chars().count(), MAX_STEM);
    }

    #[test]
    fn a_name_cannot_reach_outside_the_library() {
        for name in ["..", ".", "", "Work/../..", "a\\b", ".hidden", "_drafts"] {
            assert!(check_name(name).is_err(), "{name} should be refused");
        }
        assert!(check_name("Work Email").is_ok());
    }
}
