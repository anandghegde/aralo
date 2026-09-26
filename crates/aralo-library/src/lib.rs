//! The library folder: the only source of truth for snippets.
//!
//! Load a folder, resolve group inheritance, build the engine's snapshot, write
//! files atomically, watch for changes from elsewhere, index what is there and
//! find a snippet again, and fold a sync client's conflict copy back into the
//! file it was copied from.
//!
//! Loading never fails because of one bad file. Whatever cannot be read becomes
//! a [`Diagnostic`] and the rest of the library still works.

mod conflict;
mod index;
mod load;
mod merge;
mod search;
mod settings;
mod watch;
mod write;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aralo_engine::{Abbreviation, Rejection, Snapshot, SnapshotBuilder};
use aralo_snippet::{GroupFile, Manifest, SnippetFile, SnippetId, SnippetKind, GROUP_FILE_NAME};

pub use conflict::{is_conflict_copy, Conflict};
pub use index::{content_hash, Index, IndexError, Indexed, Recent, Stats};
pub use merge::{merge, Clashes, Merged};
pub use search::{meaning_text, Field, Hit, Query, Searcher};
pub use settings::Settings;
pub use watch::{Changes, OwnWrites, Watch, WatchError, DEFAULT_DEBOUNCE};
pub use write::write_atomic;

/// A path inside the library as the format writes it: its parts joined with
/// `/` on every platform. A report, a search result or a test reads the same
/// on Windows as on a Mac, and names what a user sees in any file manager.
pub fn slashed(path: &Path) -> String {
    path.iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Reserved at the library root for images of rich snippets (v1). The loader
/// does not walk into it and the watcher does not report changes inside it.
pub(crate) const ASSETS_FOLDER: &str = "assets";

/// Snippet files larger than this are skipped: a snippet is typed text, and a
/// stray multi-megabyte Markdown file should not stall the load.
pub const MAX_SNIPPET_BYTES: u64 = 1024 * 1024;

/// Folder nesting deeper than this is not followed.
pub const MAX_DEPTH: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("cannot read the library folder {path}: {source}")]
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Manifest {
        path: PathBuf,
        source: aralo_snippet::ParseError,
    },
    #[error("cannot write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot serialise {path}: {source}")]
    Serialise {
        path: PathBuf,
        source: aralo_snippet::ParseError,
    },
}

/// A loaded library folder.
#[derive(Debug)]
pub struct Library {
    root: PathBuf,
    manifest: Option<Manifest>,
    snippets: Vec<LoadedSnippet>,
    groups: Vec<LoadedGroup>,
    by_id: HashMap<SnippetId, usize>,
    diagnostics: Vec<Diagnostic>,
    /// Sync clients' conflict copies, each beside the file it copies.
    conflicts: Vec<Conflict>,
    /// What Aralo has saved here, so a watch on this folder can ignore its own
    /// echo. Shared with every reload of the same folder.
    writes: OwnWrites,
}

#[derive(Debug, Clone)]
pub struct LoadedSnippet {
    pub id: SnippetId,
    /// The file has no `id` yet; this one lasts until the next load.
    pub id_is_temporary: bool,
    /// Path relative to the library root.
    pub path: PathBuf,
    /// Folder names from the root down to the snippet's group.
    pub group: Vec<String>,
    pub file: SnippetFile,
    /// Trigger, case, scope and the rest, after group inheritance.
    pub settings: Settings,
    /// The hash of the file's bytes as they were read. Two loads of a file
    /// nobody touched agree on it, whatever the file's formatting.
    pub source_hash: blake3::Hash,
}

impl LoadedSnippet {
    /// What to call this snippet in a list: its label, or the first
    /// abbreviation, or the first line of the body. A hand-written file need
    /// not carry a label, and a row with no name in it is no use to anyone.
    pub fn display_name(&self) -> &str {
        if !self.file.front.label.is_empty() {
            return &self.file.front.label;
        }
        if let Some(first) = self.file.front.abbr.first() {
            return first;
        }
        self.file.body.lines().next().unwrap_or_default()
    }
}

/// One folder of the library, with its `_group.yaml` resolved.
///
/// The library root is a group too, with an empty path: it is where the
/// defaults every other group inherits are set.
#[derive(Debug, Clone)]
pub struct LoadedGroup {
    /// Folder names from the root down. Empty is the root itself.
    pub path: Vec<String>,
    /// The folder's path from the root, for writing into it.
    pub folder: PathBuf,
    /// What to call it: `name` from `_group.yaml`, else the folder's own name.
    pub name: String,
    pub colour: Option<String>,
    pub icon: Option<String>,
    /// False when this group or any group above it is switched off.
    pub enabled: bool,
    /// The file as it is on disk, so an editor can change one key and write the
    /// rest back untouched. Default when the folder has no `_group.yaml`.
    pub file: GroupFile,
    /// True when the folder has a `_group.yaml` of its own. A group without one
    /// is still a group; it just inherits everything.
    pub has_file: bool,
    /// Trigger, case, scope and the rest, as this group's snippets see them.
    pub settings: Settings,
    /// Snippets directly in this folder, not counting its sub-groups.
    pub snippets: usize,
}

/// Something in the folder that needs the user's attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Path relative to the library root.
    pub path: PathBuf,
    pub issue: Issue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    Unreadable(String),
    /// A snippet or group file that does not parse.
    Invalid(String),
    /// A `.md` file without front matter, such as a README. Not an error.
    NotASnippet,
    TooLarge {
        bytes: u64,
    },
    /// The same `id` appears in another file; this file is ignored.
    DuplicateId {
        first: PathBuf,
    },
    /// A sync client's copy of `original`, made when two machines changed it
    /// at once. It is left out of the library until it is merged back into
    /// `original` or the user decides between them.
    ConflictCopy {
        original: PathBuf,
    },
    /// The snippet works, and gets an `id` the first time Aralo saves it.
    MissingId,
    /// The snippet is loaded but this build cannot expand its `type` yet.
    UnsupportedKind(SnippetKind),
    AbbreviationRejected {
        abbreviation: String,
        reason: String,
    },
    /// Symbolic links to folders are not followed.
    SymlinkedFolder,
    TooDeep,
}

impl Library {
    /// Loads every group and snippet under `root`.
    pub fn load(root: &Path) -> Result<Self, LibraryError> {
        load::load(root, OwnWrites::new())
    }

    /// The same, keeping a record of Aralo's own writes that already exists.
    /// A [`Watch`] set up against that record ignores the saves this library
    /// makes; [`Library::load`] on its own starts a record with nothing in it.
    pub fn load_with(root: &Path, writes: OwnWrites) -> Result<Self, LibraryError> {
        load::load(root, writes)
    }

    /// Loads the same folder again. The record of Aralo's own writes carries
    /// over, so a save that is still on its way to the watcher is not mistaken
    /// for someone else's edit.
    pub fn reload(&self) -> Result<Self, LibraryError> {
        load::load(&self.root, self.writes.clone())
    }

    /// The record of Aralo's own saves. Hand a clone to [`Watch::new`].
    pub fn writes(&self) -> &OwnWrites {
        &self.writes
    }

    /// Creates `root` if needed and writes `aralo.yaml` if there is none.
    /// Existing files are never touched. Returns true when the manifest was
    /// written, which is how a caller tells a new library from an old one.
    pub fn create(root: &Path, name: Option<String>) -> Result<bool, LibraryError> {
        let write_error = |path: &Path, source| LibraryError::Write {
            path: path.to_owned(),
            source,
        };
        std::fs::create_dir_all(root).map_err(|e| write_error(root, e))?;
        let path = root.join(aralo_snippet::MANIFEST_FILE_NAME);
        if path.exists() {
            return Ok(false);
        }
        let text =
            Manifest::new(name)
                .to_file_string()
                .map_err(|source| LibraryError::Serialise {
                    path: path.clone(),
                    source,
                })?;
        write_atomic(&path, text.as_bytes()).map_err(|e| write_error(&path, e))?;
        Ok(true)
    }

    /// Writes a snippet file at `relative_path`, atomically. The caller reloads.
    ///
    /// The bytes are recorded first, so that the file system event this write
    /// provokes is recognised as Aralo's own and does not cause a second
    /// reload. Recording before writing rather than after is what makes that
    /// safe: an event can arrive while `write_atomic` is still returning.
    pub fn write_snippet(
        &self,
        relative_path: &Path,
        file: &SnippetFile,
    ) -> Result<(), LibraryError> {
        let path = self.root.join(relative_path);
        let text = file
            .to_file_string()
            .map_err(|source| LibraryError::Serialise {
                path: path.clone(),
                source,
            })?;
        self.writes.record_save(text.as_bytes());
        self.write_recorded(&path, text.as_bytes())
    }

    /// Writes `_group.yaml` for the group folder at `relative_folder`,
    /// atomically and recorded, exactly as [`Library::write_snippet`] does.
    /// The root's own group file takes an empty path.
    pub fn write_group(
        &self,
        relative_folder: &Path,
        file: &GroupFile,
    ) -> Result<(), LibraryError> {
        let path = self.root.join(relative_folder).join(GROUP_FILE_NAME);
        let text = file
            .to_file_string()
            .map_err(|source| LibraryError::Serialise {
                path: path.clone(),
                source,
            })?;
        self.write_recorded(&path, text.as_bytes())
    }

    /// Creates the folder for a group, and every folder above it. A group is a
    /// folder, so an empty one needs no file of its own.
    pub fn create_group(&self, relative_folder: &Path) -> Result<(), LibraryError> {
        let path = self.root.join(relative_folder);
        std::fs::create_dir_all(&path).map_err(|source| LibraryError::Write { path, source })
    }

    /// Removes one file, recording the removal so the watcher knows whose it
    /// was. A file that has already gone is not an error: the folder is in the
    /// state the caller asked for.
    ///
    /// The file is unlinked, not moved to the trash. A shell with somewhere
    /// kinder to put it should move the file itself and call
    /// [`Library::reload`]; the extra reload the watcher then causes is the
    /// price of not going through here.
    pub fn remove_file(&self, relative_path: &Path) -> Result<(), LibraryError> {
        let path = self.root.join(relative_path);
        self.writes.record_removal(&path);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(LibraryError::Write { path, source }),
        }
    }

    /// Removes a group folder and everything in it, recording every file it
    /// takes away. Returns the paths removed, relative to the root.
    ///
    /// This deletes the user's snippets. The caller is the one that knows
    /// whether that was asked for; [`Library::snippets`] filtered by group says
    /// how many there are to warn about.
    pub fn remove_group(&self, relative_folder: &Path) -> Result<Vec<PathBuf>, LibraryError> {
        let path = self.root.join(relative_folder);
        if relative_folder.as_os_str().is_empty() {
            return Err(LibraryError::Write {
                path,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "the library root is not a group that can be removed",
                ),
            });
        }
        let removed = self.record_removals_under(&path, relative_folder)?;
        match std::fs::remove_dir_all(&path) {
            Ok(()) => Ok(removed),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(removed),
            Err(source) => Err(LibraryError::Write { path, source }),
        }
    }

    /// Takes a file out of the library some other way than deleting it, such
    /// as moving it to the Trash, and records that it has gone so the watcher
    /// knows whose change it was. `how` gets the absolute path.
    pub fn discard_file(
        &self,
        relative_path: &Path,
        how: impl FnOnce(&Path) -> std::io::Result<()>,
    ) -> Result<(), LibraryError> {
        let path = self.root.join(relative_path);
        self.writes.record_removal(&path);
        how(&path).map_err(|source| LibraryError::Write { path, source })
    }

    /// Moves a file or a group folder inside the library, creating whatever
    /// folders the destination needs. Every file that moves is recorded gone
    /// from where it was and written where it landed, so a move the user made
    /// in Aralo does not come back through the watcher as someone else's.
    pub fn move_path(&self, from: &Path, to: &Path) -> Result<(), LibraryError> {
        let source_path = self.root.join(from);
        let target_path = self.root.join(to);
        if source_path == target_path {
            return Ok(());
        }
        if target_path.exists() {
            return Err(LibraryError::Write {
                path: target_path,
                source: std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "something is already there",
                ),
            });
        }
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| LibraryError::Write {
                path: parent.to_owned(),
                source,
            })?;
        }
        self.record_move(&source_path, &target_path)?;
        std::fs::rename(&source_path, &target_path).map_err(|source| LibraryError::Write {
            path: target_path,
            source,
        })
    }

    /// Records every file that a move is about to take from `source_path` and
    /// put at `target_path`, on both sides. Reading each file is what makes the
    /// record exact; a folder move is rare enough to afford it, and the reload
    /// that follows would read them all anyway.
    fn record_move(&self, source_path: &Path, target_path: &Path) -> Result<(), LibraryError> {
        let mut files = Vec::new();
        collect_files(source_path, &mut files).map_err(|source| LibraryError::Unreadable {
            path: source_path.to_owned(),
            source,
        })?;
        for file in files {
            let Ok(within) = file.strip_prefix(source_path) else {
                continue;
            };
            self.writes.record_removal(&file);
            if let Ok(contents) = std::fs::read(&file) {
                self.writes.record(&target_path.join(within), &contents);
            }
        }
        Ok(())
    }

    fn record_removals_under(
        &self,
        path: &Path,
        relative_folder: &Path,
    ) -> Result<Vec<PathBuf>, LibraryError> {
        let mut files = Vec::new();
        collect_files(path, &mut files).map_err(|source| LibraryError::Unreadable {
            path: path.to_owned(),
            source,
        })?;
        let mut removed = Vec::new();
        for file in &files {
            self.writes.record_removal(file);
            if let Ok(within) = file.strip_prefix(path) {
                removed.push(relative_folder.join(within));
            }
        }
        removed.sort();
        Ok(removed)
    }

    fn write_recorded(&self, path: &Path, contents: &[u8]) -> Result<(), LibraryError> {
        let write_error = |source| LibraryError::Write {
            path: path.to_owned(),
            source,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(write_error)?;
        }
        self.writes.record(path, contents);
        write_atomic(path, contents).map_err(write_error)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `None` when the folder has no `aralo.yaml`; it is still loaded.
    pub fn manifest(&self) -> Option<&Manifest> {
        self.manifest.as_ref()
    }

    pub fn snippets(&self) -> &[LoadedSnippet] {
        &self.snippets
    }

    pub fn snippet(&self, id: SnippetId) -> Option<&LoadedSnippet> {
        self.by_id.get(&id).map(|&index| &self.snippets[index])
    }

    /// Every folder of the library in tree order, the root first.
    pub fn groups(&self) -> &[LoadedGroup] {
        &self.groups
    }

    /// One group by its path from the root. An empty path is the root.
    pub fn group(&self, path: &[String]) -> Option<&LoadedGroup> {
        self.groups.iter().find(|group| group.path == path)
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Every conflict copy in the folder, in path order of the copies. Each is
    /// also reported as an [`Issue::ConflictCopy`].
    pub fn conflicts(&self) -> &[Conflict] {
        &self.conflicts
    }

    /// True when `id` has a conflict copy waiting. Its file is not a settled
    /// version while that lasts, so it must not become a merge base.
    pub fn in_conflict(&self, id: SnippetId) -> bool {
        self.conflicts.iter().any(|conflict| conflict.id == id)
    }

    /// The engine's view of the library: every abbreviation of every enabled
    /// snippet this build can expand. Rejected abbreviations come back as
    /// diagnostics.
    pub fn snapshot(&self) -> (Snapshot, Vec<Diagnostic>) {
        let mut builder = SnapshotBuilder::new();
        for snippet in &self.snippets {
            if !snippet.settings.enabled || snippet.file.front.kind != SnippetKind::Text {
                continue;
            }
            for text in &snippet.file.front.abbr {
                let settings = &snippet.settings;
                builder.add(Abbreviation {
                    snippet_id: aralo_engine::SnippetId(snippet.id.as_u128()),
                    text: text.clone(),
                    trigger: settings.trigger,
                    case: settings.case,
                    whole_word: settings.whole_word,
                    keep_delimiter: settings.keep_delimiter,
                    delimiters: settings.delimiters.clone(),
                    scope: settings.scope.clone(),
                });
            }
        }
        let (snapshot, rejections) = builder.build();
        let diagnostics = rejections
            .into_iter()
            .map(|rejection| self.rejection_diagnostic(rejection))
            .collect();
        (snapshot, diagnostics)
    }

    fn rejection_diagnostic(&self, rejection: Rejection) -> Diagnostic {
        let id = SnippetId::from_u128(rejection.snippet_id.0);
        Diagnostic {
            path: self
                .snippet(id)
                .map(|snippet| snippet.path.clone())
                .unwrap_or_default(),
            issue: Issue::AbbreviationRejected {
                abbreviation: rejection.text,
                reason: format!("{:?}", rejection.reason),
            },
        }
    }
}

/// Every file under `path`, following no symbolic links to folders. A path that
/// is a file is the one entry; a path that is not there yields none.
fn collect_files(path: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return Ok(());
    };
    if !metadata.is_dir() {
        out.push(path.to_owned());
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        collect_files(&entry?.path(), out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_path_is_written_with_slashes_on_every_platform() {
        let path = Path::new("Work").join("Email").join("signature.md");
        assert_eq!(slashed(&path), "Work/Email/signature.md");
        assert_eq!(slashed(Path::new("note.md")), "note.md");
        assert_eq!(slashed(Path::new("")), "");
    }
}
