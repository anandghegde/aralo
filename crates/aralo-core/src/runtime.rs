//! The library, running: the folder watched, the index kept in step, and the
//! shell told when something changed.
//!
//! [`Core`] is a value that is read and written on whatever thread holds it.
//! `Runtime` is that value with the two background jobs a running app needs
//! around it, and it is what a shell holds for the lifetime of the app:
//!
//! - **the watch.** Someone edits a snippet in another editor, or a sync client
//!   brings one down. The folder is the truth, so Aralo reads it again and the
//!   shell is told. Aralo's own writes are recognised by their contents and do
//!   not come back around ([`aralo_library::OwnWrites`]).
//! - **the indexer.** One thread owns the only connection that writes to the
//!   index, so a rebuild cannot collide with a save. Readers get a second
//!   connection and never wait for it.
//!
//! The index is a cache: a runtime whose index cannot be opened still watches,
//! still expands and still saves. It loses recents and usage counts until the
//! next start, which is a worse menu, not lost work.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use aralo_engine::Snapshot;
use aralo_library::{Changes, Index, Indexed, Query, Recent, Stats, Watch, DEFAULT_DEBOUNCE};
use aralo_snippet::SnippetId;

use crate::{state, Core, CoreError, SearchHit};

/// What the shell is told about, as it happens.
///
/// A listener is called on a background thread and should return quickly: the
/// watch cannot report the next change until it does. A shell that draws
/// something should hand the change to its main thread and return.
///
/// `library` is the library as it now stands, under a read lock for the length
/// of the call. Read what is needed and return: calling back into the
/// [`Runtime`] to change the library from in here would wait for a lock this
/// call is holding.
pub trait LibraryListener: Send + Sync + 'static {
    fn changed(&self, change: LibraryChange, library: &Core);
}

/// Something that happened to the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryChange {
    /// Something outside Aralo changed the folder and it has been read again.
    /// The paths are relative to the library root.
    Outside { paths: Vec<PathBuf> },
    /// Aralo changed the folder, through [`Runtime::edit`].
    Edited,
    /// The folder was read again because something asked for it, through
    /// [`Runtime::reload`].
    Reloaded,
    /// The folder changed and reading it failed. The library in memory is the
    /// last one that loaded, and expansion carries on with it.
    Failed { message: String },
    /// The index caught up with the library.
    Indexed {
        added: usize,
        updated: usize,
        removed: usize,
    },
}

/// How a runtime is put together. The defaults are what the app uses.
#[derive(Debug, Clone)]
pub struct RuntimeOptions {
    /// Where the search index file goes. `None` is the place Aralo keeps
    /// caches for this library ([`state::index_path`]); a machine that will not
    /// say where the user's home is runs without an index.
    pub index: Option<PathBuf>,
    /// Watch the folder for changes made outside Aralo.
    pub watch: bool,
    /// How long the folder must be quiet before a change is reported. Tests use
    /// a short one; everything else wants [`DEFAULT_DEBOUNCE`].
    pub debounce: Duration,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            index: None,
            watch: true,
            debounce: DEFAULT_DEBOUNCE,
        }
    }
}

/// A library that is open, watched and indexed.
///
/// Dropping it stops both threads and waits for them, so the index is closed
/// with its last write finished.
pub struct Runtime {
    /// Held for its `Drop`, and named first because fields drop in the order
    /// they are declared: the watch stops before the indexer does, so no
    /// change is reported to a thread that has already gone.
    _watch: Option<Watch>,
    core: Arc<RwLock<Core>>,
    /// The connection reads are served from. The indexer writes through its
    /// own, so a read never waits for a rebuild: SQLite in WAL mode allows one
    /// writer and any number of readers at once.
    reader: Option<Mutex<Index>>,
    indexer: Option<Indexer>,
    listener: Arc<dyn LibraryListener>,
    /// Why there is no index, when there is none and there should have been.
    index_error: Option<String>,
}

impl Runtime {
    /// Opens the library at `root` and starts watching and indexing it.
    ///
    /// Returns as soon as the library is loaded. The first index build happens
    /// on the indexer thread, so a large library does not hold up the app; the
    /// listener hears [`LibraryChange::Indexed`] when it is done.
    pub fn open<L: LibraryListener>(
        root: &Path,
        options: RuntimeOptions,
        listener: L,
    ) -> Result<Self, CoreError> {
        Self::with_core(Core::open(root)?, options, Arc::new(listener))
    }

    /// The same, for a library that is already open.
    pub fn with_core(
        core: Core,
        options: RuntimeOptions,
        listener: Arc<dyn LibraryListener>,
    ) -> Result<Self, CoreError> {
        let root = core.root().to_owned();
        let writes = core.library().writes().clone();
        let core = Arc::new(RwLock::new(core));

        // An index that will not open is a cache that will not open. The
        // library is what matters and it is already loaded, so the runtime
        // carries on without one and says why when asked.
        let path = options.index.or_else(|| state::index_path(&root));
        let opened = path.map(|path| open_index(&path, &core, &listener));
        let (reader, indexer, index_error) = match opened {
            Some(Ok((reader, indexer))) => (Some(reader), Some(indexer), None),
            Some(Err(error)) => (None, None, Some(error.to_string())),
            None => (
                None,
                None,
                Some("this machine will not say where the user's home folder is".to_owned()),
            ),
        };

        let watch = if options.watch {
            let core = Arc::clone(&core);
            let listener = Arc::clone(&listener);
            let jobs = indexer.as_ref().map(Indexer::sender);
            Some(Watch::with_debounce(
                &root,
                writes,
                options.debounce,
                move |changes: Changes| {
                    let reloaded = write(&core).reload();
                    let change = match reloaded {
                        Ok(()) => {
                            if let Some(jobs) = &jobs {
                                let _ = jobs.send(Job::Sync);
                            }
                            LibraryChange::Outside {
                                paths: changes.paths,
                            }
                        }
                        Err(error) => LibraryChange::Failed {
                            message: error.to_string(),
                        },
                    };
                    listener.changed(change, &read(&core));
                },
            )?)
        } else {
            None
        };

        Ok(Self {
            _watch: watch,
            core,
            reader,
            indexer,
            listener,
            index_error,
        })
    }

    /// Reads the library. The engine's tap thread holds nothing but the
    /// snapshot, so this is for the shell's own reads.
    pub fn read<T>(&self, with: impl FnOnce(&Core) -> T) -> T {
        with(&read(&self.core))
    }

    /// Changes the library, then brings the index and the shell up to date.
    ///
    /// The closure is where the editing calls go:
    ///
    /// ```no_run
    /// # use aralo_core::{Draft, Runtime, RuntimeOptions};
    /// # fn example(runtime: &Runtime, draft: &Draft) -> Result<(), aralo_core::CoreError> {
    /// let id = runtime.edit(|core| core.create_snippet(&["Work".to_owned()], draft))?;
    /// # let _ = id;
    /// # Ok(())
    /// # }
    /// ```
    pub fn edit<T>(&self, change: impl FnOnce(&mut Core) -> T) -> T {
        let outcome = change(&mut write(&self.core));
        self.reindex();
        self.announce(LibraryChange::Edited);
        outcome
    }

    /// Reads the folder again, as if something outside had changed it. A shell
    /// offers this as "reload", for a folder that arrived while Aralo was not
    /// looking.
    pub fn reload(&self) -> Result<(), CoreError> {
        write(&self.core).reload()?;
        self.reindex();
        self.announce(LibraryChange::Reloaded);
        Ok(())
    }

    /// What the engine matches against. Cheap to clone and safe to hold: the
    /// tap thread keeps one and swaps it when the shell hands it a new one.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        read(&self.core).snapshot()
    }

    pub fn root(&self) -> PathBuf {
        read(&self.core).root().to_owned()
    }

    /// Searches the loaded library. This does not touch the index: the snippets
    /// are already in memory, and the matcher is what makes the ranking.
    pub fn search(&self, query: &Query) -> Vec<SearchHit> {
        read(&self.core).search(query)
    }

    /// Records that a snippet was expanded, for the recents list.
    ///
    /// It is queued, not written here: this is called from the injector the
    /// moment an expansion lands, and that thread waits for nothing.
    pub fn record_expansion(&self, id: SnippetId) {
        if let Some(indexer) = &self.indexer {
            let _ = indexer.jobs.send(Job::Expansion {
                id,
                at: SystemTime::now(),
            });
        }
    }

    /// The snippets used most recently, most recent first. Empty when there is
    /// no index.
    pub fn recents(&self, limit: usize) -> Result<Vec<Recent>, CoreError> {
        match &self.reader {
            Some(index) => Ok(lock(index).recents(limit)?),
            None => Ok(Vec::new()),
        }
    }

    /// How often one snippet has been expanded, and when it last was.
    pub fn stats(&self, id: SnippetId) -> Result<Stats, CoreError> {
        match &self.reader {
            Some(index) => Ok(lock(index).stats(id)?),
            None => Ok(Stats::default()),
        }
    }

    /// Whether this runtime has an index at all. A shell can say why recents
    /// are empty instead of looking broken.
    pub fn has_index(&self) -> bool {
        self.reader.is_some()
    }

    /// Why there is no index, when there is none. `None` means there is one.
    pub fn index_error(&self) -> Option<&str> {
        self.index_error.as_deref()
    }

    /// Waits until the indexer has finished everything queued before this call.
    ///
    /// Tests use it. So does a shell that wants to close knowing the last
    /// expansion was counted.
    pub fn flush(&self) {
        let Some(indexer) = &self.indexer else { return };
        let (done, wait) = mpsc::channel();
        if indexer.jobs.send(Job::Flush(done)).is_ok() {
            // An error means the indexer has stopped, and a stopped indexer has
            // nothing left to wait for.
            let _ = wait.recv();
        }
    }

    /// Tells the listener, with the library the change left behind. The write
    /// lock is released by now: the listener reads under a read lock, and a
    /// listener that had to wait for the writer would be a listener that could
    /// deadlock against the caller.
    fn announce(&self, change: LibraryChange) {
        self.listener.changed(change, &read(&self.core));
    }

    fn reindex(&self) {
        if let Some(indexer) = &self.indexer {
            let _ = indexer.jobs.send(Job::Sync);
        }
    }
}

impl std::fmt::Debug for Runtime {
    /// The library is the interesting part and it is behind a lock this must
    /// not wait for, so what is printed is the shape of the runtime around it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("watching", &self._watch.is_some())
            .field("indexer", &self.indexer)
            .field("index_error", &self.index_error)
            .finish()
    }
}

/// Opens the index and starts the thread that writes to it.
///
/// Both connections are opened here, one after the other, so that whichever of
/// them finds an index from an older version and rebuilds the schema does it
/// before the other looks.
fn open_index(
    path: &Path,
    core: &Arc<RwLock<Core>>,
    listener: &Arc<dyn LibraryListener>,
) -> Result<(Mutex<Index>, Indexer), CoreError> {
    let writer = Index::open(path)?;
    let reader = Index::open(path)?;
    let indexer = Indexer::start(writer, Arc::clone(core), Arc::clone(listener))?;
    Ok((Mutex::new(reader), indexer))
}

/// The thread that owns the index's writing connection.
struct Indexer {
    jobs: Sender<Job>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Debug)]
enum Job {
    /// Bring the index to what the library says now.
    Sync,
    Expansion {
        id: SnippetId,
        at: SystemTime,
    },
    /// Reply on this channel once everything queued before it is done.
    Flush(Sender<()>),
}

impl Indexer {
    fn start(
        mut index: Index,
        core: Arc<RwLock<Core>>,
        listener: Arc<dyn LibraryListener>,
    ) -> Result<Self, CoreError> {
        let (jobs, queue) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("aralo-index".to_owned())
            .spawn({
                move || {
                    // The library on disk moved on while Aralo was not running,
                    // so the first thing the index does is catch up with it.
                    sync(&mut index, &core, &listener);
                    work(&mut index, &core, &queue, &listener);
                }
            })
            .map_err(|source| CoreError::Thread { source })?;
        Ok(Self {
            jobs,
            thread: Some(thread),
        })
    }

    fn sender(&self) -> Sender<Job> {
        self.jobs.clone()
    }
}

impl std::fmt::Debug for Indexer {
    /// A listener belongs to the shell and says nothing useful here; what
    /// matters is whether the thread is still running.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Indexer")
            .field("running", &self.thread.is_some())
            .finish()
    }
}

impl Drop for Indexer {
    fn drop(&mut self) {
        // Dropping the last sender is what ends the loop; the thread is joined
        // so the connection closes with its last write finished.
        let (dead, _) = mpsc::channel();
        drop(std::mem::replace(&mut self.jobs, dead));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn work(
    index: &mut Index,
    core: &RwLock<Core>,
    queue: &Receiver<Job>,
    listener: &Arc<dyn LibraryListener>,
) {
    while let Ok(job) = queue.recv() {
        match job {
            Job::Sync => sync(index, core, listener),
            Job::Expansion { id, at } => {
                // A count that does not get written is a count, not a snippet.
                let _ = index.record_expansion(id, at);
            }
            Job::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

fn sync(index: &mut Index, core: &RwLock<Core>, listener: &Arc<dyn LibraryListener>) {
    // The lock is held for the length of the sync, which is why the indexer
    // reads the library rather than being handed a copy of it: copying every
    // snippet on every save would cost more than the wait.
    let outcome = index.sync(read(core).library());
    let change = match outcome {
        Ok(indexed) if indexed.changed() => {
            let Indexed {
                added,
                updated,
                removed,
                ..
            } = indexed;
            LibraryChange::Indexed {
                added,
                updated,
                removed,
            }
        }
        Ok(_) => return,
        Err(error) => LibraryChange::Failed {
            message: error.to_string(),
        },
    };
    listener.changed(change, &read(core));
}

/// The three locks, with poisoning treated as what it is: a thread panicked
/// somewhere else. Refusing to expand for the rest of the session would be a
/// worse answer than carrying on, and the library is re-read from the folder
/// anyway.
fn read(core: &RwLock<Core>) -> RwLockReadGuard<'_, Core> {
    core.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write(core: &RwLock<Core>) -> RwLockWriteGuard<'_, Core> {
    core.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock(index: &Mutex<Index>) -> MutexGuard<'_, Index> {
    index
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
