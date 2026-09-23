//! Conflict copies: the second file a sync client leaves when two machines
//! changed one snippet before either saw the other's change.
//!
//! Every client names its copy after the original, in its own way. A name alone
//! is not enough to go on — `Invoice 2.md` may be a snippet the user made on
//! purpose — so a file is taken for a conflict copy only when three things hold:
//! it sits in the same folder as the original, it carries the same `id`, and
//! its name is one a sync client gives a copy of the original's name. Anything
//! else with a contested `id` stays a [`Issue::DuplicateId`](crate::Issue).
//!
//! A file with no `id` is never paired: its identity comes from its path, so
//! two files cannot share one.

use std::path::{Path, PathBuf};

use aralo_snippet::SnippetId;

/// A conflict copy and the file it is a copy of, both relative to the library
/// root. The original is the one the library loads; the copy is left out of the
/// library until it is merged or the user resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub id: SnippetId,
    pub original: PathBuf,
    pub copy: PathBuf,
}

/// True when `copy` is the name a sync client gives to a conflicting copy of
/// `original`: the same folder, and a file stem that is the original's with a
/// client's suffix after it.
pub fn is_conflict_copy(copy: &Path, original: &Path) -> bool {
    if copy.parent() != original.parent() || copy == original {
        return false;
    }
    if copy.extension() != original.extension() {
        return false;
    }
    let (Some(copy), Some(original)) = (
        copy.file_stem().and_then(|stem| stem.to_str()),
        original.file_stem().and_then(|stem| stem.to_str()),
    ) else {
        return false;
    };
    copy.strip_prefix(original).is_some_and(is_copy_suffix)
}

/// What each client appends to the original's name.
fn is_copy_suffix(suffix: &str) -> bool {
    // Dropbox and Nextcloud: `name (conflicted copy 2026-09-23)`, with the
    // user's name in front when Dropbox knows it, and a time after it when
    // Nextcloud writes it. Dropbox also has `name (Case Conflict)`.
    if let Some(inner) = suffix
        .strip_prefix(" (")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let lower = inner.to_lowercase();
        if lower.contains("conflicted copy") || lower.starts_with("case conflict") {
            return true;
        }
        // Google Drive and Box: `name (1)`.
        return is_number(inner);
    }
    // iCloud Drive: `name 2`. It never numbers a copy 1.
    if let Some(number) = suffix.strip_prefix(' ') {
        return is_number(number) && number != "0" && number != "1";
    }
    // Syncthing: `name.sync-conflict-20260923-101500-ABCDEFG`.
    if suffix.starts_with(".sync-conflict-") {
        return true;
    }
    // ownCloud: `name_conflict-20260923-101500`.
    if suffix.starts_with("_conflict-") {
        return true;
    }
    // OneDrive: `name-MACHINENAME`. A machine name has no spaces or dots.
    if let Some(machine) = suffix.strip_prefix('-') {
        return !machine.is_empty()
            && machine
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    }
    false
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.len() <= 4 && text.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(copy: &str, original: &str) -> bool {
        is_conflict_copy(Path::new(copy), Path::new(original))
    }

    #[test]
    fn each_clients_naming_is_recognised() {
        for name in [
            "Work/sig (conflicted copy 2026-09-23).md",
            "Work/sig (Sam Lee's conflicted copy 2026-09-23).md",
            "Work/sig (conflicted copy 2026-09-23 101500).md",
            "Work/sig (Case Conflict).md",
            "Work/sig (Case Conflict 1).md",
            "Work/sig (1).md",
            "Work/sig 2.md",
            "Work/sig 12.md",
            "Work/sig.sync-conflict-20260923-101500-ABCDEFG.md",
            "Work/sig_conflict-20260923-101500.md",
            "Work/sig-MacBook-Pro.md",
        ] {
            assert!(copy(name, "Work/sig.md"), "{name}");
        }
    }

    #[test]
    fn a_name_that_only_looks_related_is_not_a_copy() {
        for (name, original) in [
            ("Work/sig.md", "Work/sig.md"),
            ("Other/sig 2.md", "Work/sig.md"),
            ("Work/sig 1.md", "Work/sig.md"),
            ("Work/sig 2.txt", "Work/sig.md"),
            ("Work/signature.md", "Work/sig.md"),
            ("Work/sig copy.md", "Work/sig.md"),
            ("Work/sig (draft).md", "Work/sig.md"),
            ("Work/sig-old notes.md", "Work/sig.md"),
            ("Work/sig-.md", "Work/sig.md"),
            ("Work/sig (12345).md", "Work/sig.md"),
        ] {
            assert!(!copy(name, original), "{name}");
        }
    }

    #[test]
    fn a_name_with_brackets_of_its_own_still_pairs() {
        assert!(copy(
            "Replies (old) (conflicted copy 2026-09-23).md",
            "Replies (old).md"
        ));
        assert!(copy("Replies (old) 2.md", "Replies (old).md"));
    }
}
