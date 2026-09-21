use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use aralo_snippet::{
    GroupFile, Manifest, ParseError, SnippetFile, SnippetId, SnippetKind, GROUP_FILE_NAME,
    MANIFEST_FILE_NAME, SNIPPET_EXTENSION,
};

use crate::{
    Diagnostic, Issue, Library, LibraryError, LoadedGroup, LoadedSnippet, OwnWrites, Settings,
    ASSETS_FOLDER, MAX_DEPTH, MAX_SNIPPET_BYTES,
};

pub(crate) fn load(root: &Path, writes: OwnWrites) -> Result<Library, LibraryError> {
    // Fail early and clearly when the folder itself is the problem.
    fs::read_dir(root).map_err(|source| LibraryError::Unreadable {
        path: root.to_owned(),
        source,
    })?;

    let manifest_path = root.join(MANIFEST_FILE_NAME);
    let manifest = match fs::read_to_string(&manifest_path) {
        Ok(text) => Some(
            Manifest::parse(&text).map_err(|source| LibraryError::Manifest {
                path: manifest_path,
                source,
            })?,
        ),
        Err(_) => None,
    };

    let mut loader = Loader {
        root,
        snippets: Vec::new(),
        groups: Vec::new(),
        diagnostics: Vec::new(),
    };
    loader.folder(Path::new(""), &Settings::default(), &[]);

    let Loader {
        mut snippets,
        mut groups,
        mut diagnostics,
        ..
    } = loader;

    // The first file in path order keeps a contested ID, whatever the file
    // system's listing order was.
    snippets.sort_by(|a, b| a.path.cmp(&b.path));
    let mut by_id: HashMap<SnippetId, usize> = HashMap::new();
    let mut kept: Vec<LoadedSnippet> = Vec::with_capacity(snippets.len());
    for snippet in snippets {
        if let Some(&first) = by_id.get(&snippet.id) {
            diagnostics.push(Diagnostic {
                path: snippet.path,
                issue: Issue::DuplicateId {
                    first: kept[first].path.clone(),
                },
            });
            continue;
        }
        by_id.insert(snippet.id, kept.len());
        kept.push(snippet);
    }
    diagnostics.sort_by(|a, b| a.path.cmp(&b.path));

    // The root first, then the rest in tree order, so a list drawn straight
    // from this reads the way the folder does.
    groups.sort_by(|a, b| a.path.cmp(&b.path));
    for group in &mut groups {
        group.snippets = kept
            .iter()
            .filter(|snippet| snippet.group == group.path)
            .count();
    }

    Ok(Library {
        root: root.to_owned(),
        manifest,
        snippets: kept,
        groups,
        by_id,
        diagnostics,
        writes,
    })
}

struct Loader<'a> {
    root: &'a Path,
    snippets: Vec<LoadedSnippet>,
    groups: Vec<LoadedGroup>,
    diagnostics: Vec<Diagnostic>,
}

impl Loader<'_> {
    fn report(&mut self, path: &Path, issue: Issue) {
        self.diagnostics.push(Diagnostic {
            path: path.to_owned(),
            issue,
        });
    }

    /// `relative` is the folder's path from the root; `inherited` is what its
    /// parent resolves to.
    fn folder(&mut self, relative: &Path, inherited: &Settings, group: &[String]) {
        if group.len() > MAX_DEPTH {
            self.report(relative, Issue::TooDeep);
            return;
        }
        let absolute = self.root.join(relative);

        let group_path = relative.join(GROUP_FILE_NAME);
        let (file, has_file, settings) = match fs::read_to_string(absolute.join(GROUP_FILE_NAME)) {
            Ok(text) => match GroupFile::parse(&text) {
                Ok(file) => {
                    let settings = inherited.for_group(&file);
                    (file, true, settings)
                }
                Err(error) => {
                    // A broken group file must not change how snippets behave
                    // in surprising ways: fall back to what the parent says.
                    self.report(&group_path, Issue::Invalid(error.to_string()));
                    (GroupFile::default(), true, inherited.clone())
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (GroupFile::default(), false, inherited.clone())
            }
            Err(error) => {
                self.report(&group_path, Issue::Unreadable(error.to_string()));
                (GroupFile::default(), true, inherited.clone())
            }
        };
        self.groups.push(LoadedGroup {
            name: file
                .name
                .clone()
                .or_else(|| group.last().cloned())
                .unwrap_or_default(),
            path: group.to_vec(),
            folder: PathBuf::from(relative),
            colour: file.colour.clone(),
            icon: file.icon.clone(),
            enabled: settings.enabled,
            has_file,
            file,
            settings: settings.clone(),
            // Filled once every snippet is loaded and the contested IDs are out.
            snippets: 0,
        });

        let entries = match fs::read_dir(&absolute) {
            Ok(entries) => entries,
            Err(error) => {
                self.report(relative, Issue::Unreadable(error.to_string()));
                return;
            }
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with('.') || name.starts_with('_') {
                continue;
            }
            let path = relative.join(name);
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let is_folder = if file_type.is_symlink() {
                match fs::metadata(entry.path()) {
                    Ok(target) if target.is_dir() => {
                        self.report(&path, Issue::SymlinkedFolder);
                        continue;
                    }
                    Ok(_) => false,
                    Err(_) => continue,
                }
            } else {
                file_type.is_dir()
            };

            if is_folder {
                if group.is_empty() && name == ASSETS_FOLDER {
                    continue;
                }
                let mut inner = group.to_vec();
                inner.push(name.to_owned());
                self.folder(&path, &settings, &inner);
            } else if Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(SNIPPET_EXTENSION))
            {
                self.snippet(&path, &settings, group);
            }
        }
    }

    fn snippet(&mut self, relative: &Path, group_settings: &Settings, group: &[String]) {
        let absolute = self.root.join(relative);
        match fs::metadata(&absolute) {
            Ok(metadata) if metadata.len() > MAX_SNIPPET_BYTES => {
                self.report(
                    relative,
                    Issue::TooLarge {
                        bytes: metadata.len(),
                    },
                );
                return;
            }
            Ok(_) => {}
            Err(error) => {
                self.report(relative, Issue::Unreadable(error.to_string()));
                return;
            }
        }
        let text = match fs::read_to_string(&absolute) {
            Ok(text) => text,
            Err(error) => {
                self.report(relative, Issue::Unreadable(error.to_string()));
                return;
            }
        };
        let file = match SnippetFile::parse(&text) {
            Ok(file) => file,
            Err(ParseError::MissingFrontMatter) => {
                self.report(relative, Issue::NotASnippet);
                return;
            }
            Err(error) => {
                self.report(relative, Issue::Invalid(error.to_string()));
                return;
            }
        };

        if file.front.kind != SnippetKind::Text {
            self.report(relative, Issue::UnsupportedKind(file.front.kind));
        }
        let (id, id_is_temporary) = match file.front.id {
            Some(id) => (id, false),
            None => {
                self.report(relative, Issue::MissingId);
                (temporary_id(relative), true)
            }
        };
        self.snippets.push(LoadedSnippet {
            id,
            id_is_temporary,
            path: PathBuf::from(relative),
            group: group.to_vec(),
            settings: group_settings.for_snippet(&file.front),
            file,
        });
    }
}

/// The identity of a snippet whose file carries none.
///
/// A file with no `id` has no durable identity beyond where it sits, so its ID
/// is derived from its path. That makes it the same on every load, which keeps
/// the index from treating a file nobody touched as a new snippet each time,
/// and it changes when the file moves, which is the whole truth about a file
/// with nothing else to go on. The first save Aralo makes writes a real ULID
/// and this is never used for that snippet again.
fn temporary_id(relative: &Path) -> SnippetId {
    let hash = blake3::hash(relative.as_os_str().as_encoded_bytes());
    let bytes: [u8; 16] = hash.as_bytes()[..16]
        .try_into()
        .expect("blake3 produces 32 bytes");
    SnippetId::from_u128(u128::from_be_bytes(bytes))
}
