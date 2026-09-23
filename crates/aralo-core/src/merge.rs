//! Folding a sync client's conflict copy back into the snippet it copies
//! ([ADR-0006]).
//!
//! Two machines change one snippet before either sees the other's change, and
//! the sync client keeps both: the original, and a copy beside it with a name
//! of its own making. The library loads the original and leaves the copy out
//! ([`aralo_library::Conflict`]). Here the two are merged against the version
//! this machine last saw settled, the merge base. A clean merge is written over
//! the original and the copy is discarded; one that is not leaves both files
//! where they are for the user to decide between.
//!
//! Discarding is the shell's to do, because the shell knows where the platform
//! keeps things a user may want back. [`SetAside`] is what happens when it
//! says nothing: the copy moves into Aralo's own folder, out of the library.
//!
//! A copy that will not merge waits for the user, who is shown both sides
//! ([`Core::conflict`]) and picks one, or writes the version to keep
//! ([`Core::resolve_conflict`]). Either way the copy is discarded as a clean
//! merge's is.
//!
//! [ADR-0006]: ../../../docs/adr/0006-files-as-source-of-truth.md

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use aralo_library::{merge, Clashes, Conflict, Merged};
use aralo_snippet::{SnippetFile, SnippetId};

use crate::{Core, CoreError};

/// Takes a merged conflict copy out of the library folder, somewhere the user
/// can still find it: the Trash, on a Mac.
pub trait Discard: Send + Sync + std::fmt::Debug {
    /// Moves the file at the absolute path `path` out of the library.
    fn discard(&self, path: &Path) -> std::io::Result<()>;
}

/// Moves each discarded copy into one folder, named after the moment it was
/// moved so that two copies of one name do not collide.
#[derive(Debug, Clone)]
pub struct SetAside {
    folder: PathBuf,
}

impl SetAside {
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        Self {
            folder: folder.into(),
        }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }
}

impl Discard for SetAside {
    fn discard(&self, path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.folder)?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or_default();
        let mut target = self.folder.join(format!("{millis} {name}"));
        let mut attempt = 1;
        while target.exists() {
            attempt += 1;
            target = self.folder.join(format!("{millis}-{attempt} {name}"));
        }
        match std::fs::rename(path, &target) {
            Ok(()) => Ok(()),
            // The library can be on another volume from Aralo's folder, and a
            // rename cannot cross one.
            Err(_) => {
                std::fs::copy(path, &target)?;
                std::fs::remove_file(path)
            }
        }
    }
}

/// What one pass over the conflict copies did, with paths relative to the
/// library root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// Copies merged into their originals and discarded.
    pub merged: Vec<PathBuf>,
    /// Copies left beside their originals, because both sides changed the same
    /// thing or the copy would not read.
    pub unresolved: Vec<PathBuf>,
}

impl MergeReport {
    pub fn is_empty(&self) -> bool {
        self.merged.is_empty() && self.unresolved.is_empty()
    }
}

/// A conflict copy the user has to decide about, with everything a resolver
/// shows: both files as they are on disk, the version both were edited from
/// when there is one, and what the two changed differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictSides {
    pub conflict: Conflict,
    /// The original's text.
    pub original: String,
    /// The copy's text.
    pub copy: String,
    /// The merge base's text, when this machine has one.
    pub base: Option<String>,
    /// What both sides changed. Empty when the copy would not read, which
    /// `copy_problem` then explains, or when the two now merge cleanly.
    pub clashes: Clashes,
    /// Why the copy is not a snippet file, when it is not one.
    pub copy_problem: Option<String>,
}

/// What the user decided about a conflict copy.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// The original stays as it is.
    KeepOriginal,
    /// The copy's version goes into the original's file.
    KeepCopy,
    /// This text, a whole snippet file, goes into the original's file.
    Write(String),
}

impl Core {
    /// The conflict copies waiting in the folder, each with its original.
    pub fn conflicts(&self) -> &[Conflict] {
        self.library.conflicts()
    }

    /// Merges every conflict copy that merges cleanly into its original and
    /// discards it, then reads the folder again if anything changed.
    ///
    /// `base` answers with the merge base of a snippet, when there is one. The
    /// original is written before its copies are discarded, so a failure
    /// between the two leaves a copy that merges again, cleanly, next time.
    pub fn merge_conflicts(
        &mut self,
        base: impl Fn(SnippetId) -> Option<SnippetFile>,
        discard: &dyn Discard,
    ) -> Result<MergeReport, CoreError> {
        let mut report = MergeReport::default();
        // Every copy of one original, merged into it one after another.
        let mut by_original: BTreeMap<PathBuf, Vec<Conflict>> = BTreeMap::new();
        for conflict in self.library.conflicts() {
            by_original
                .entry(conflict.original.clone())
                .or_default()
                .push(conflict.clone());
        }
        let mut changed = false;

        for group in by_original.into_values() {
            let id = group[0].id;
            let Some(loaded) = self.library.snippet(id) else {
                continue;
            };
            let settled = base(id);
            let mut ours = loaded.file.clone();
            let mut folded = Vec::new();
            for conflict in &group {
                let theirs = std::fs::read_to_string(self.root().join(&conflict.copy))
                    .ok()
                    .and_then(|text| SnippetFile::parse(&text).ok());
                match theirs.map(|theirs| merge(settled.as_ref(), &ours, &theirs)) {
                    Some(Merged::Clean(merged)) => {
                        ours = *merged;
                        folded.push(conflict.copy.clone());
                    }
                    Some(Merged::Conflicted(_)) | None => {
                        report.unresolved.push(conflict.copy.clone());
                    }
                }
            }
            if folded.is_empty() {
                continue;
            }
            if ours != loaded.file {
                let path = loaded.path.clone();
                self.library.write_snippet(&path, &ours)?;
            }
            for copy in folded {
                self.library
                    .discard_file(&copy, |path| discard.discard(path))?;
                report.merged.push(copy);
            }
            changed = true;
        }

        if changed {
            self.reload()?;
        }
        report.merged.sort();
        report.unresolved.sort();
        Ok(report)
    }

    /// Both sides of the conflict copy at `copy`, a path relative to the
    /// library root. `base` answers as it does for
    /// [`Core::merge_conflicts`].
    pub fn conflict(
        &self,
        copy: &Path,
        base: impl Fn(SnippetId) -> Option<SnippetFile>,
    ) -> Result<ConflictSides, CoreError> {
        let conflict = self.find_conflict(copy)?;
        let original = self.read_file(&conflict.original)?;
        let copy = self.read_file(&conflict.copy)?;
        let settled = base(conflict.id);

        let (clashes, copy_problem) =
            match (SnippetFile::parse(&original), SnippetFile::parse(&copy)) {
                (_, Err(error)) => (Clashes::default(), Some(error.to_string())),
                (Ok(ours), Ok(theirs)) => match merge(settled.as_ref(), &ours, &theirs) {
                    Merged::Clean(_) => (Clashes::default(), None),
                    Merged::Conflicted(clashes) => (clashes, None),
                },
                // The original loaded, so it parses; a file changed since the
                // folder was read is shown as it is and decided about as it is.
                (Err(_), Ok(_)) => (Clashes::default(), None),
            };
        let base = settled.and_then(|file| file.to_file_string().ok());

        Ok(ConflictSides {
            conflict,
            original,
            copy,
            base,
            clashes,
            copy_problem,
        })
    }

    /// Settles the conflict copy at `copy` the way the user decided, discards
    /// the copy, and reads the folder again.
    ///
    /// The original is written before the copy goes, as in a merge, so a
    /// failure between the two leaves the user's decision in the original and
    /// the copy still asking about it.
    pub fn resolve_conflict(
        &mut self,
        copy: &Path,
        resolution: Resolution,
        discard: &dyn Discard,
    ) -> Result<(), CoreError> {
        let conflict = self.find_conflict(copy)?;
        let parse = |text: &str| {
            SnippetFile::parse(text).map_err(|error| CoreError::NotASnippet(error.to_string()))
        };
        let keep = match resolution {
            Resolution::KeepOriginal => None,
            Resolution::KeepCopy => Some(parse(&self.read_file(&conflict.copy)?)?),
            Resolution::Write(text) => Some(parse(&text)?),
        };
        if let Some(mut file) = keep {
            // The file stays the snippet it was, whatever the kept text says.
            file.front.id = Some(conflict.id);
            self.library.write_snippet(&conflict.original, &file)?;
        }
        self.library
            .discard_file(&conflict.copy, |path| discard.discard(path))?;
        self.reload()
    }

    fn read_file(&self, relative: &Path) -> Result<String, CoreError> {
        let path = self.root().join(relative);
        std::fs::read_to_string(&path).map_err(|source| CoreError::Read { path, source })
    }

    fn find_conflict(&self, copy: &Path) -> Result<Conflict, CoreError> {
        self.library
            .conflicts()
            .iter()
            .find(|conflict| conflict.copy == copy)
            .cloned()
            .ok_or_else(|| CoreError::NoSuchConflict(copy.display().to_string()))
    }
}
