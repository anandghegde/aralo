//! Moving the library to another folder, such as one iCloud Drive or Dropbox
//! syncs (plan 5.2).
//!
//! The folder is the user's work and the only copy Aralo knows of, so a move
//! never deletes anything. It copies, checks, and only then lets the caller
//! switch:
//!
//! 1. **Check the destination.** It is empty or does not exist, it is not
//!    inside the library and the library is not inside it.
//! 2. **Check the source.** Every file is on this machine: a file a sync
//!    client keeps only in the cloud is refused by name rather than copied as
//!    a placeholder, or downloaded behind the user's back.
//! 3. **Copy into a staging folder** beside the destination, a hidden name on
//!    the same volume, flushing each file to disk.
//! 4. **Verify.** Every file is read back from both sides and compared by
//!    hash; a file that changed while it was being copied is copied again,
//!    once. Then the copy is loaded as a library and must hold the same
//!    snippets, with the same content, as the original.
//! 5. **Rename the staging folder into place.** Until this step the
//!    destination either does not exist or is still empty, so a failure
//!    anywhere before it leaves nothing that looks like a library; the staging
//!    folder is removed.
//! 6. **Carry the index over.** It is keyed by the library's path, so the
//!    moved library would otherwise start with no recents, no usage counts and
//!    no merge bases ([`Index::copy_to`]). A failure here costs a cache, not a
//!    snippet, and is reported rather than undone.
//!
//! The old folder stays where it was. The caller switches the running app over
//! (the bridge's `move_library` stops the old watch and indexer and starts new
//! ones), asks [`Moved::changed_since`] whether anything reached the old folder
//! in the meantime, and only then offers to put it in the Trash.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use aralo_library::{content_hash, icloud_placeholder, Index, Library};

use crate::state;

/// A file macOS keeps only in the cloud: `SF_DATALESS` in `st_flags`. Reading
/// it would ask the provider to download it, which can fail offline or take as
/// long as the network does.
#[cfg(target_os = "macos")]
const SF_DATALESS: u32 = 0x4000_0000;

/// How many times a file that keeps changing under the copy is copied again
/// before the move gives up.
const RETRIES: usize = 1;

/// Names that are never copied: Finder's view settings, which the destination
/// makes its own, and the temporary file of an atomic write that was under way.
fn skipped(name: &str) -> bool {
    name == ".DS_Store" || name.ends_with(".aralo-tmp")
}

#[derive(Debug, thiserror::Error)]
pub enum MoveError {
    #[error("the library is already at {0}")]
    SameFolder(PathBuf),
    #[error("{to} is inside the library; choose a folder outside it")]
    InsideLibrary { to: PathBuf },
    #[error("the library is inside {to}; choose a folder that is not above it")]
    ContainsLibrary { to: PathBuf },
    #[error("{0} already holds files; choose an empty folder, or use it as the library as it is")]
    NotEmpty(PathBuf),
    #[error("{} files are in the cloud and not on this Mac, starting with {}; download them first", paths.len(), paths.first().map(|path| aralo_library::slashed(path)).unwrap_or_default())]
    NotDownloaded { paths: Vec<PathBuf> },
    #[error("{} is a link this platform cannot copy as a link", .0.display())]
    Link(PathBuf),
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("{} kept changing while it was copied; try again when the sync client is idle", aralo_library::slashed(.0))]
    KeptChanging(PathBuf),
    #[error("the copy does not load as the same library: {0}")]
    Mismatch(String),
    #[error(transparent)]
    Library(#[from] aralo_library::LibraryError),
}

/// What is at a folder the user picked, for a picker to say what choosing it
/// would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    /// The sync client that looks after it, when the path says so.
    pub provider: Option<Provider>,
    pub contents: Contents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contents {
    /// Nothing there, or an empty folder: the library can be moved here.
    Empty,
    /// An Aralo library already: another Mac's, synced here, or one moved
    /// here before. It can be used as it is.
    Library { snippets: usize },
    /// Other files. A library is neither moved into it nor opened in it.
    Occupied,
}

/// The sync clients whose folders Aralo recognises by path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    ICloudDrive,
    Dropbox,
    OneDrive,
    GoogleDrive,
    Box,
    /// Some other File Provider under `~/Library/CloudStorage`.
    CloudStorage,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::ICloudDrive => "iCloud Drive",
            Provider::Dropbox => "Dropbox",
            Provider::OneDrive => "OneDrive",
            Provider::GoogleDrive => "Google Drive",
            Provider::Box => "Box",
            Provider::CloudStorage => "a cloud storage provider",
        }
    }

    /// Which client syncs `path`, going by where it is: iCloud Drive's
    /// `Library/Mobile Documents`, a File Provider's `Library/CloudStorage/`
    /// folder, or a folder with Dropbox's `.dropbox` marker at its top, which
    /// is where the older Dropbox client keeps `~/Dropbox`.
    pub fn of(path: &Path) -> Option<Provider> {
        let parts: Vec<String> = path
            .components()
            .filter_map(|part| match part {
                Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        for window in parts.windows(2) {
            if window[0] == "Library" && window[1] == "Mobile Documents" {
                return Some(Provider::ICloudDrive);
            }
        }
        for window in parts.windows(3) {
            if window[0] == "Library" && window[1] == "CloudStorage" {
                let folder = window[2].as_str();
                let provider = if folder == "Dropbox" || folder.starts_with("Dropbox-") {
                    Provider::Dropbox
                } else if folder.starts_with("OneDrive") {
                    Provider::OneDrive
                } else if folder.starts_with("GoogleDrive") {
                    Provider::GoogleDrive
                } else if folder.starts_with("Box") {
                    Provider::Box
                } else {
                    Provider::CloudStorage
                };
                return Some(provider);
            }
        }
        path.ancestors()
            .any(|folder| folder.join(".dropbox").is_file())
            .then_some(Provider::Dropbox)
    }
}

/// What choosing `path` as the library's folder would mean.
pub fn inspect(path: &Path) -> Location {
    let contents = if !path.exists() || is_empty_folder(path).unwrap_or(false) {
        Contents::Empty
    } else if path.join(aralo_snippet::MANIFEST_FILE_NAME).is_file() {
        let snippets = Library::load(path)
            .map(|library| library.snippets().len())
            .unwrap_or(0);
        Contents::Library { snippets }
    } else {
        Contents::Occupied
    };
    Location {
        path: path.to_owned(),
        provider: Provider::of(path),
        contents,
    }
}

/// A library that was copied to its new folder and checked there.
#[derive(Debug, Clone)]
pub struct Moved {
    pub from: PathBuf,
    pub to: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub snippets: usize,
    /// Why the index did not come along, when it did not. The moved library
    /// then builds a new one, without recents, counts or merge bases.
    pub index_problem: Option<String>,
    /// Every file copied, relative to the root, and the hash it was copied
    /// with.
    copied: BTreeMap<PathBuf, blake3::Hash>,
}

impl Moved {
    /// Files in the old folder that are not what was copied: changed, added or
    /// gone since. Empty means the old folder holds nothing the new one lacks,
    /// and may go to the Trash. Anything else is work that arrived during the
    /// move — a sync client's delivery, a save in another editor — and the old
    /// folder must stay until the user has looked.
    pub fn changed_since(&self) -> Vec<PathBuf> {
        let now = match scan(&self.from) {
            Ok(scan) => scan,
            // An old folder that cannot be read cannot be said to be safe to
            // throw away.
            Err(_) => return vec![PathBuf::new()],
        };
        let present: std::collections::BTreeSet<&PathBuf> = now
            .files
            .iter()
            .filter(|(_, entry)| *entry == Entry::File)
            .map(|(path, _)| path)
            .collect();
        let mut changed: Vec<PathBuf> = now.evicted.clone();
        for path in &present {
            let same = self
                .copied
                .get(*path)
                .is_some_and(|hash| hash_file(&self.from.join(path)).ok().as_ref() == Some(hash));
            if !same {
                changed.push((*path).clone());
            }
        }
        changed.extend(
            self.copied
                .keys()
                .filter(|path| !present.contains(path))
                .cloned(),
        );
        changed.sort();
        changed.dedup();
        changed
    }
}

/// Copies the library at `from` to `to`, checks the copy, and carries the
/// index kept for it in `cache` along. `from` is left as it was. See the
/// module documentation for the steps and what a failure at each leaves.
pub fn move_library(from: &Path, to: &Path, cache: Option<&Path>) -> Result<Moved, MoveError> {
    let from_canonical = from.canonicalize().map_err(|source| MoveError::Read {
        path: from.to_owned(),
        source,
    })?;
    let to = absolute(to);
    check_destination(&from_canonical, &to)?;

    let source = scan(from)?;
    if !source.evicted.is_empty() {
        return Err(MoveError::NotDownloaded {
            paths: source.evicted,
        });
    }
    let original = Library::load(from)?;

    let parent = to.parent().unwrap_or(Path::new("/")).to_owned();
    fs::create_dir_all(&parent).map_err(|source| MoveError::Write {
        path: parent.clone(),
        source,
    })?;
    let staging = parent.join(format!(
        ".{}.{}.aralo-moving",
        to.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Aralo".to_owned()),
        std::process::id()
    ));
    // A staging folder left by a move that crashed is Aralo's own and holds
    // nothing that is not still in the library.
    let _ = fs::remove_dir_all(&staging);

    let copied = match copy_and_verify(from, &staging, &source, &original) {
        Ok(copied) => copied,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    if let Err(error) = put_in_place(&staging, &to) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    let index_problem = cache.and_then(|cache| carry_index(cache, from, &to).err());
    Ok(Moved {
        from: from.to_owned(),
        to,
        files: copied.len(),
        bytes: source.bytes,
        snippets: original.snippets().len(),
        index_problem,
        copied,
    })
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map(|here| here.join(path))
            .unwrap_or_else(|_| path.to_owned())
    }
}

/// The destination resolved as far as it exists, so that a symbolic link in
/// its path (`/tmp`, a home folder on another volume) cannot hide that it is
/// inside the library.
fn resolved(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut existing = path;
    loop {
        if let Ok(canonical) = existing.canonicalize() {
            let mut out = canonical;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                existing = parent;
            }
            _ => return path.to_owned(),
        }
    }
}

fn check_destination(from: &Path, to: &Path) -> Result<(), MoveError> {
    let target = resolved(to);
    if target == from {
        return Err(MoveError::SameFolder(to.to_owned()));
    }
    if target.starts_with(from) {
        return Err(MoveError::InsideLibrary { to: to.to_owned() });
    }
    if from.starts_with(&target) {
        return Err(MoveError::ContainsLibrary { to: to.to_owned() });
    }
    if to.exists()
        && !is_empty_folder(to).map_err(|source| MoveError::Read {
            path: to.to_owned(),
            source,
        })?
    {
        return Err(MoveError::NotEmpty(to.to_owned()));
    }
    Ok(())
}

/// A folder with nothing in it but Finder's `.DS_Store`. A file is not a
/// folder, so it is not empty either.
fn is_empty_folder(path: &Path) -> io::Result<bool> {
    if !path.is_dir() {
        return Ok(false);
    }
    for entry in fs::read_dir(path)? {
        let name = entry?.file_name();
        if !skipped(&name.to_string_lossy()) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    Folder,
    File,
    Link,
}

/// Everything under a library root, relative to it, in the order it is to be
/// created: a folder before what is in it.
#[derive(Debug, Default)]
struct Scan {
    files: Vec<(PathBuf, Entry)>,
    /// Files that are in the cloud and not on this machine.
    evicted: Vec<PathBuf>,
    bytes: u64,
}

fn scan(root: &Path) -> Result<Scan, MoveError> {
    let mut out = Scan::default();
    walk(root, Path::new(""), &mut out)?;
    out.evicted.sort();
    Ok(out)
}

fn walk(root: &Path, relative: &Path, out: &mut Scan) -> Result<(), MoveError> {
    let folder = root.join(relative);
    let entries = fs::read_dir(&folder).map_err(|source| MoveError::Read {
        path: folder.clone(),
        source,
    })?;
    let mut names: Vec<_> = entries
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<_, _>>()
        .map_err(|source| MoveError::Read {
            path: folder.clone(),
            source,
        })?;
    names.sort();
    for name in names {
        let text = name.to_string_lossy();
        if skipped(&text) {
            continue;
        }
        let path = relative.join(&name);
        if let Some(original) = icloud_placeholder(&text) {
            out.evicted.push(relative.join(original));
            continue;
        }
        let absolute = root.join(&path);
        let metadata = fs::symlink_metadata(&absolute).map_err(|source| MoveError::Read {
            path: absolute.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            out.files.push((path, Entry::Link));
        } else if metadata.is_dir() {
            out.files.push((path.clone(), Entry::Folder));
            walk(root, &path, out)?;
        } else if is_dataless(&metadata) {
            out.evicted.push(path);
        } else {
            out.bytes += metadata.len();
            out.files.push((path, Entry::File));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn is_dataless(metadata: &fs::Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    metadata.st_flags() & SF_DATALESS != 0
}

#[cfg(not(target_os = "macos"))]
fn is_dataless(_metadata: &fs::Metadata) -> bool {
    false
}

fn hash_file(path: &Path) -> io::Result<blake3::Hash> {
    let mut hasher = blake3::Hasher::new();
    let mut file = fs::File::open(path)?;
    io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize())
}

fn copy_and_verify(
    from: &Path,
    staging: &Path,
    source: &Scan,
    original: &Library,
) -> Result<BTreeMap<PathBuf, blake3::Hash>, MoveError> {
    let write_error = |path: &Path| {
        let path = path.to_owned();
        move |source| MoveError::Write { path, source }
    };
    fs::create_dir(staging).map_err(write_error(staging))?;
    for (path, entry) in &source.files {
        let (there, here) = (from.join(path), staging.join(path));
        match entry {
            Entry::Folder => fs::create_dir(&here).map_err(write_error(&here))?,
            Entry::File => copy_file(&there, &here)?,
            Entry::Link => copy_link(&there, &here)?,
        }
    }

    let mut copied = BTreeMap::new();
    for (path, entry) in &source.files {
        if *entry != Entry::File {
            continue;
        }
        let (there, here) = (from.join(path), staging.join(path));
        let mut tries = 0;
        let hash = loop {
            let original = hash_file(&there).map_err(|source| MoveError::Read {
                path: there.clone(),
                source,
            })?;
            let copy = hash_file(&here).map_err(|source| MoveError::Read {
                path: here.clone(),
                source,
            })?;
            if original == copy {
                break copy;
            }
            if tries == RETRIES {
                return Err(MoveError::KeptChanging(path.clone()));
            }
            tries += 1;
            copy_file(&there, &here)?;
        };
        copied.insert(path.clone(), hash);
    }

    // The files match byte for byte; this is the check that the copy is the
    // same library as well, which is what the user will be expanding from.
    let copy = Library::load(staging)?;
    let before = fingerprint(original);
    let after = fingerprint(&copy);
    if before != after {
        let missing = before
            .iter()
            .find(|row| !after.contains(row))
            .or_else(|| after.iter().find(|row| !before.contains(row)))
            .map(|(_, path, _)| path.clone())
            .unwrap_or_default();
        return Err(MoveError::Mismatch(format!(
            "{} snippets before, {} after; {} differs",
            before.len(),
            after.len(),
            missing
        )));
    }
    Ok(copied)
}

/// Each snippet as the index would see it: id, path and content.
fn fingerprint(library: &Library) -> Vec<(String, String, String)> {
    let mut rows: Vec<_> = library
        .snippets()
        .iter()
        .map(|snippet| {
            (
                snippet.id.to_string(),
                aralo_library::slashed(&snippet.path),
                content_hash(snippet),
            )
        })
        .collect();
    rows.sort();
    rows
}

/// Copies a file with its permissions and flushes it, so the verification
/// reads what is on the disk.
fn copy_file(from: &Path, to: &Path) -> Result<(), MoveError> {
    fs::copy(from, to).map_err(|source| MoveError::Write {
        path: to.to_owned(),
        source,
    })?;
    fs::File::open(to)
        .and_then(|file| file.sync_all())
        .map_err(|source| MoveError::Write {
            path: to.to_owned(),
            source,
        })
}

#[cfg(unix)]
fn copy_link(from: &Path, to: &Path) -> Result<(), MoveError> {
    let target = fs::read_link(from).map_err(|source| MoveError::Read {
        path: from.to_owned(),
        source,
    })?;
    std::os::unix::fs::symlink(target, to).map_err(|source| MoveError::Write {
        path: to.to_owned(),
        source,
    })
}

#[cfg(not(unix))]
fn copy_link(from: &Path, _to: &Path) -> Result<(), MoveError> {
    Err(MoveError::Link(from.to_owned()))
}

/// Renames the checked copy to where the user asked for it. An empty folder
/// already there is removed first; it was checked empty, and Finder may have
/// put a `.DS_Store` in it since.
fn put_in_place(staging: &Path, to: &Path) -> Result<(), MoveError> {
    if to.exists() {
        if !is_empty_folder(to).unwrap_or(false) {
            return Err(MoveError::NotEmpty(to.to_owned()));
        }
        fs::remove_dir_all(to).map_err(|source| MoveError::Write {
            path: to.to_owned(),
            source,
        })?;
    }
    fs::rename(staging, to).map_err(|source| MoveError::Write {
        path: to.to_owned(),
        source,
    })
}

/// Copies the index kept for `from` to where the one for `to` belongs. Any
/// index already there is an older library's at that path and is replaced.
fn carry_index(cache: &Path, from: &Path, to: &Path) -> Result<(), String> {
    let old = state::index_in(cache, from);
    if !old.exists() {
        return Ok(());
    }
    let new = state::index_in(cache, to);
    for suffix in ["", "-wal", "-shm"] {
        let mut name = new.clone().into_os_string();
        name.push(suffix);
        let _ = fs::remove_file(PathBuf::from(name));
    }
    let index = Index::open(&old).map_err(|error| error.to_string())?;
    index.copy_to(&new).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_are_known_by_where_their_folders_are() {
        let home = Path::new("/Users/sam");
        let cases = [
            (
                "Library/Mobile Documents/com~apple~CloudDocs/Aralo",
                Some(Provider::ICloudDrive),
            ),
            (
                "Library/CloudStorage/Dropbox/Aralo",
                Some(Provider::Dropbox),
            ),
            (
                "Library/CloudStorage/Dropbox-Work/Aralo",
                Some(Provider::Dropbox),
            ),
            (
                "Library/CloudStorage/OneDrive-Personal/Aralo",
                Some(Provider::OneDrive),
            ),
            (
                "Library/CloudStorage/GoogleDrive-sam@example.com/My Drive/Aralo",
                Some(Provider::GoogleDrive),
            ),
            ("Library/CloudStorage/Box-Box/Aralo", Some(Provider::Box)),
            (
                "Library/CloudStorage/pCloud/Aralo",
                Some(Provider::CloudStorage),
            ),
            ("Aralo", None),
            ("Documents/Library/Aralo", None),
        ];
        for (path, provider) in cases {
            assert_eq!(Provider::of(&home.join(path)), provider, "{path}");
        }
    }

    #[test]
    fn only_a_folder_with_nothing_but_finder_settings_is_empty() {
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join("x");
        assert!(!is_empty_folder(&path).unwrap());
        fs::create_dir(&path).unwrap();
        assert!(is_empty_folder(&path).unwrap());
        fs::write(path.join(".DS_Store"), b"").unwrap();
        assert!(is_empty_folder(&path).unwrap());
        fs::write(path.join("notes.txt"), b"").unwrap();
        assert!(!is_empty_folder(&path).unwrap());
    }
}
