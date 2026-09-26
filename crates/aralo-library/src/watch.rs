//! Watching the library folder, and telling Aralo's own writes from everyone
//! else's.
//!
//! A library folder is shared ground. The user edits a snippet in Aralo, in a
//! text editor and through a sync client, and all three write the same files
//! ([ADR-0006]). The watcher turns those writes into one debounced list of
//! paths relative to the library root, which the caller reloads and reindexes.
//!
//! Aralo's own saves reach the watcher too, and reloading the library because
//! Aralo just saved it is work with no result. Suppressing by path alone would
//! be wrong: it would swallow a real edit that lands on the same file in the
//! same moment. So a write is remembered by content. [`OwnWrites::record`]
//! stores the hash of what Aralo wrote, and an event is dropped only when the
//! file still holds exactly those bytes. A different hash, or a file Aralo
//! never wrote, is reported.
//!
//! [ADR-0006]: ../../../docs/adr/0006-files-as-source-of-truth.md

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use aralo_snippet::{GROUP_FILE_NAME, MANIFEST_FILE_NAME, SNIPPET_EXTENSION};
use notify::{EventKind, RecursiveMode, Watcher as _};

use crate::ASSETS_FOLDER;

/// How long the folder must be quiet before a batch is reported. One save in a
/// text editor is often several file system events, and a sync client can
/// rewrite a folder file by file.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(200);

/// How long a recorded write stays claimable. FSEvents is not prompt, so this
/// is generous. It can afford to be: a stale record only ever suppresses an
/// event for a file that still holds exactly the bytes Aralo wrote, and there
/// is nothing in that to reindex.
const WRITE_MEMORY: Duration = Duration::from_secs(30);

/// How often the watcher thread wakes to notice it has been dropped.
const POLL: Duration = Duration::from_millis(50);

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("cannot watch {path}: {source}")]
    Start {
        path: PathBuf,
        source: notify::Error,
    },
    #[error("cannot start the watcher thread: {source}")]
    Thread { source: std::io::Error },
}

/// The writes Aralo made itself, so the watcher can recognise its own echo.
///
/// Cloning shares one record. Keep one beside the library that saves files and
/// hand a clone to the watcher.
#[derive(Debug, Clone, Default)]
pub struct OwnWrites {
    pending: Arc<Mutex<HashMap<PathBuf, Vec<Write>>>>,
    /// Every snippet file Aralo saved since the library was opened, by the
    /// hash of its bytes. Unlike `pending` this is never spent or forgotten:
    /// the index asks it which files are this machine's own edits, which must
    /// not become a merge base ([`crate::Index::sync`]).
    saved: Arc<Mutex<HashSet<blake3::Hash>>>,
}

#[derive(Debug)]
struct Write {
    /// What Aralo put there, or `None` when it took the file away. Absence is
    /// content too, and recording it is what keeps a delete from looking like
    /// someone else's.
    hash: Option<blake3::Hash>,
    at: Instant,
}

impl OwnWrites {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remembers that Aralo has just written `contents` to the absolute path
    /// `path`.
    pub fn record(&self, path: &Path, contents: &[u8]) {
        self.push(path, Some(blake3::hash(contents)));
    }

    /// Remembers that the snippet file Aralo has just written held
    /// `contents`, for as long as this record lasts.
    pub fn record_save(&self, contents: &[u8]) {
        if let Ok(mut saved) = self.saved.lock() {
            saved.insert(blake3::hash(contents));
        }
    }

    /// True when Aralo saved a snippet file holding exactly the bytes that
    /// hash to `hash` since this record began.
    pub fn saved(&self, hash: &blake3::Hash) -> bool {
        self.saved.lock().is_ok_and(|saved| saved.contains(hash))
    }

    /// Remembers that Aralo is about to remove `path`, so the event that
    /// removal provokes is recognised as Aralo's own. The record is honoured
    /// only while the file really is gone: if anything puts one back at that
    /// path, the event describes someone else's file and is reported.
    pub fn record_removal(&self, path: &Path) {
        self.push(path, None);
    }

    fn push(&self, path: &Path, hash: Option<blake3::Hash>) {
        // A poisoned lock means a panic inside one of these short critical
        // sections. There is nothing to recover, and the only cost of giving
        // up is one reindex that was not needed.
        let Ok(mut pending) = self.pending.lock() else {
            return;
        };
        expire(&mut pending);
        pending.entry(key(path)).or_default().push(Write {
            hash,
            at: Instant::now(),
        });
    }

    /// How many writes are still waiting for their event.
    pub fn outstanding(&self) -> usize {
        let Ok(mut pending) = self.pending.lock() else {
            return 0;
        };
        expire(&mut pending);
        pending.values().map(Vec::len).sum()
    }

    /// True when `path` still holds exactly what Aralo wrote there, or is gone
    /// and Aralo is what took it away. Either makes the event that brought us
    /// here Aralo's own. The record is spent: a second event for the same write
    /// is reported, because by then the bytes on disk are someone else's doing.
    fn claim(&self, path: &Path) -> bool {
        let path = key(path);
        let hash = fs::read(&path).ok().map(|contents| blake3::hash(&contents));
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        expire(&mut pending);
        let Some(writes) = pending.get_mut(&path) else {
            return false;
        };
        let Some(index) = writes.iter().position(|write| write.hash == hash) else {
            return false;
        };
        // Anything written before the match was overwritten by it, so its
        // event, if one is still coming, is not worth waiting for.
        writes.drain(..=index);
        if writes.is_empty() {
            pending.remove(&path);
        }
        true
    }
}

/// What a write is filed under. The saver names the file through the library
/// root it was given; the watcher names it the way the platform resolved it.
/// On macOS those differ whenever a folder above it is a symbolic link, which
/// covers both `/tmp` and any home folder that has been moved, so the two
/// sides have to agree on a spelling before they can agree on a file.
///
/// Only the folder is resolved, never the file: a snippet that is itself a
/// symbolic link is reported under its own name, not its target's.
fn key(path: &Path) -> PathBuf {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return path.to_owned();
    };
    match fs::canonicalize(folder) {
        Ok(folder) => folder.join(name),
        // The folder is gone, so nothing will be read from it either.
        Err(_) => path.to_owned(),
    }
}

fn expire(pending: &mut HashMap<PathBuf, Vec<Write>>) {
    pending.retain(|_, writes| {
        writes.retain(|write| write.at.elapsed() < WRITE_MEMORY);
        !writes.is_empty()
    });
}

/// What changed in the library folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    /// Paths relative to the library root, sorted and free of duplicates. A
    /// path that no longer exists was deleted.
    pub paths: Vec<PathBuf>,
    /// True when a `_group.yaml` or the manifest is among them. Those reach
    /// snippets that did not change themselves, through inheritance, so the
    /// caller reloads the whole library rather than these paths alone.
    pub groups_changed: bool,
}

/// A running watch on a library folder. Dropping it stops the watch.
#[derive(Debug)]
pub struct Watch {
    // Held only so that dropping `Watch` drops the watcher and ends the watch.
    _watcher: notify::RecommendedWatcher,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Watch {
    /// Watches `root` and its sub-folders, calling `on_change` on a thread of
    /// its own once the folder has been quiet for [`DEFAULT_DEBOUNCE`].
    pub fn new<F>(root: &Path, writes: OwnWrites, on_change: F) -> Result<Self, WatchError>
    where
        F: FnMut(Changes) + Send + 'static,
    {
        Self::with_debounce(root, writes, DEFAULT_DEBOUNCE, on_change)
    }

    /// The same, with the quiet period given. Tests use a short one.
    pub fn with_debounce<F>(
        root: &Path,
        writes: OwnWrites,
        debounce: Duration,
        mut on_change: F,
    ) -> Result<Self, WatchError>
    where
        F: FnMut(Changes) + Send + 'static,
    {
        let start_error = |source| WatchError::Start {
            path: root.to_owned(),
            source,
        };
        let (sender, receiver) = mpsc::channel::<PathBuf>();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else {
                    return;
                };
                if !reports_a_change(&event.kind) {
                    return;
                }
                for path in event.paths {
                    // The receiver is gone once the watch has been dropped.
                    if sender.send(path).is_err() {
                        return;
                    }
                }
            })
            .map_err(start_error)?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(start_error)?;

        let roots = roots_of(root);
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("aralo-library-watch".to_owned())
            .spawn({
                let stop = Arc::clone(&stop);
                move || collect(&receiver, &roots, &writes, debounce, &stop, &mut on_change)
            })
            .map_err(|source| WatchError::Thread { source })?;

        Ok(Self {
            _watcher: watcher,
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn collect<F>(
    receiver: &mpsc::Receiver<PathBuf>,
    roots: &[PathBuf],
    writes: &OwnWrites,
    debounce: Duration,
    stop: &AtomicBool,
    on_change: &mut F,
) where
    F: FnMut(Changes),
{
    let mut batch: BTreeSet<PathBuf> = BTreeSet::new();
    let mut newest: Option<Instant> = None;
    let poll = POLL.min(debounce);
    while !stop.load(Ordering::Relaxed) {
        match receiver.recv_timeout(poll) {
            Ok(path) => {
                batch.insert(path);
                newest = Some(Instant::now());
            }
            Err(RecvTimeoutError::Timeout) => {}
            // The watcher has been dropped; nothing more is coming.
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if newest.is_some_and(|at| at.elapsed() >= debounce) {
            newest = None;
            report(&mut batch, roots, writes, on_change);
        }
    }
}

fn report<F>(
    batch: &mut BTreeSet<PathBuf>,
    roots: &[PathBuf],
    writes: &OwnWrites,
    on_change: &mut F,
) where
    F: FnMut(Changes),
{
    let mut paths = Vec::new();
    let mut groups_changed = false;
    for absolute in std::mem::take(batch) {
        let Some(relative) = roots
            .iter()
            .find_map(|root| absolute.strip_prefix(root).ok())
        else {
            continue;
        };
        let Some(kind) = kind_of(relative) else {
            continue;
        };
        if writes.claim(&absolute) {
            continue;
        }
        groups_changed |= kind != Kind::Snippet;
        paths.push(relative.to_owned());
    }
    if paths.is_empty() {
        return;
    }
    paths.sort();
    on_change(Changes {
        paths,
        groups_changed,
    });
}

/// What a changed path is to the library. `None` for anything the loader would
/// not read anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Snippet,
    Group,
    Manifest,
}

fn kind_of(relative: &Path) -> Option<Kind> {
    let name = relative.file_name()?.to_str()?;
    let folders = relative.parent()?;
    // A snippet under a hidden or underscored folder is not loaded, so a write
    // to it is not a change to the library. Neither is anything under the
    // reserved `assets` folder at the root.
    if folders.components().any(|part| {
        part.as_os_str()
            .to_str()
            .is_none_or(|part| part.starts_with('.') || part.starts_with('_'))
    }) || folders.starts_with(ASSETS_FOLDER)
    {
        return None;
    }
    if name == GROUP_FILE_NAME {
        return Some(Kind::Group);
    }
    if name == MANIFEST_FILE_NAME && folders.as_os_str().is_empty() {
        return Some(Kind::Manifest);
    }
    // The loader ignores hidden and underscored names, and the temporary file
    // `write_atomic` renames into place starts with a dot.
    if name.starts_with('.') || name.starts_with('_') {
        return None;
    }
    Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(SNIPPET_EXTENSION))
        .then_some(Kind::Snippet)
}

/// The prefixes a reported path may carry. FSEvents resolves symbolic links
/// before it reports, and on macOS both a temporary folder and a home folder
/// can sit under one, so the path that arrives is not the path handed in.
fn roots_of(root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![root.to_owned()];
    match fs::canonicalize(root) {
        Ok(canonical) if canonical != root => roots.push(canonical),
        _ => {}
    }
    roots
}

fn reports_a_change(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_is_claimed_once_and_only_for_its_own_bytes() {
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join("note.md");
        fs::write(&path, b"saved by aralo").unwrap();

        let writes = OwnWrites::new();
        writes.record(&path, b"saved by aralo");
        assert_eq!(writes.outstanding(), 1);
        assert!(writes.claim(&path), "the file holds what Aralo wrote");
        assert_eq!(writes.outstanding(), 0);
        assert!(!writes.claim(&path), "the record is spent");
    }

    #[test]
    fn an_edit_that_lands_on_top_of_a_save_is_not_claimed() {
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join("note.md");
        let writes = OwnWrites::new();

        writes.record(&path, b"saved by aralo");
        // Someone else got there between the save and its event.
        fs::write(&path, b"edited in a text editor").unwrap();
        assert!(!writes.claim(&path));
    }

    #[test]
    fn a_superseded_save_does_not_claim_a_later_event() {
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join("note.md");
        let writes = OwnWrites::new();

        writes.record(&path, b"first");
        writes.record(&path, b"second");
        fs::write(&path, b"second").unwrap();
        // Claiming the second write drops the first: its event, if it is still
        // coming, describes bytes that are no longer on disk.
        assert!(writes.claim(&path));
        assert_eq!(writes.outstanding(), 0);
    }

    #[test]
    fn a_missing_file_is_never_claimed_for_a_write() {
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join("gone.md");
        let writes = OwnWrites::new();
        writes.record(&path, b"anything");
        assert!(!writes.claim(&path));
    }

    #[test]
    fn a_removal_aralo_made_is_claimed_once() {
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join("note.md");
        fs::write(&path, b"going").unwrap();

        let writes = OwnWrites::new();
        writes.record_removal(&path);
        fs::remove_file(&path).unwrap();
        assert!(writes.claim(&path), "Aralo is what took the file away");
        assert!(!writes.claim(&path), "the record is spent");
    }

    #[test]
    fn a_file_put_back_where_aralo_deleted_one_is_someone_elses() {
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join("note.md");
        let writes = OwnWrites::new();

        writes.record_removal(&path);
        // A sync client delivered the file again before the event arrived.
        fs::write(&path, b"restored from the cloud").unwrap();
        assert!(!writes.claim(&path));
    }

    #[test]
    fn only_files_the_loader_reads_are_a_change() {
        assert_eq!(kind_of(Path::new("note.md")), Some(Kind::Snippet));
        assert_eq!(
            kind_of(Path::new("Work/Email/note.MD")),
            Some(Kind::Snippet)
        );
        assert_eq!(kind_of(Path::new("Work/_group.yaml")), Some(Kind::Group));
        assert_eq!(kind_of(Path::new("_group.yaml")), Some(Kind::Group));
        assert_eq!(kind_of(Path::new("aralo.yaml")), Some(Kind::Manifest));

        // Not the library: a manifest below the root, the temporary file of an
        // atomic write, an editor's backup, a folder the loader skips, and
        // anything that is not a snippet.
        assert_eq!(kind_of(Path::new("Work/aralo.yaml")), None);
        assert_eq!(kind_of(Path::new(".note.md.913.aralo-tmp")), None);
        assert_eq!(kind_of(Path::new("Work/.note.md.swp")), None);
        assert_eq!(kind_of(Path::new("_drafts/note.md")), None);
        assert_eq!(kind_of(Path::new(".git/note.md")), None);
        assert_eq!(kind_of(Path::new("assets/logo.md")), None);
        assert_eq!(kind_of(Path::new("README.txt")), None);
        assert_eq!(kind_of(Path::new("Work")), None);
    }
}
