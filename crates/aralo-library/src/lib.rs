//! The library folder: the only source of truth for snippets.
//!
//! This is the M1 slice of the crate: load a folder, resolve group
//! inheritance, build the engine's snapshot, and write files atomically. The
//! watcher, the SQLite index, search and conflict merging arrive in M2.
//!
//! Loading never fails because of one bad file. Whatever cannot be read becomes
//! a [`Diagnostic`] and the rest of the library still works.

mod load;
mod settings;
mod write;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aralo_engine::{Abbreviation, Rejection, Snapshot, SnapshotBuilder};
use aralo_snippet::{Manifest, SnippetFile, SnippetId, SnippetKind};

pub use settings::Settings;
pub use write::write_atomic;

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
    by_id: HashMap<SnippetId, usize>,
    diagnostics: Vec<Diagnostic>,
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
        load::load(root)
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
        let write_error = |source| LibraryError::Write {
            path: path.clone(),
            source,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(write_error)?;
        }
        write_atomic(&path, text.as_bytes()).map_err(write_error)
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

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
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
