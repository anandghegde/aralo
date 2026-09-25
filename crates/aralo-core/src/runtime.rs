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
//!   connection and never wait for it. When the runtime has an embedding
//!   model and AI is on, the same thread loads it, embeds what the index has
//!   no vector for, and hands search the library's meaning
//!   ([`crate::meaning`]). The model follows the AI switch
//!   ([`Runtime::use_model`]): off, it is dropped and never loaded.
//!
//! Every time the folder is read, the conflict copies a sync client left in it
//! are merged against their bases in the index, and the ones that merge
//! cleanly are discarded ([`crate::merge`]). The ones that do not stay where
//! they are, reported as [`aralo_library::Issue::ConflictCopy`].
//!
//! The index is a cache: a runtime whose index cannot be opened still watches,
//! still expands and still saves. It loses recents, usage counts and search by
//! meaning until the next start, which is a worse menu, not lost work.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use aralo_embed::Model;
use aralo_engine::Snapshot;
use aralo_library::{Changes, Index, Indexed, Query, Recent, Stats, Watch, DEFAULT_DEBOUNCE};
use aralo_snippet::SnippetId;

use crate::ai::{AiSettings, Refusal};
use crate::meaning::Plan;
use crate::merge::{ConflictSides, Discard, Resolution, SetAside};
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
    /// Conflict copies a sync client left were merged into their originals
    /// and discarded. The paths are the copies', relative to the library root.
    Merged { copies: Vec<PathBuf> },
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
    /// Where a conflict copy goes once it is merged. `None` sets it aside in
    /// the place Aralo keeps things for this library
    /// ([`state::set_aside_path`]); a machine that will not say where that is
    /// leaves conflict copies unmerged.
    pub discard: Option<Arc<dyn Discard>>,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            index: None,
            watch: true,
            debounce: DEFAULT_DEBOUNCE,
            discard: None,
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
    reader: Option<Arc<Mutex<Index>>>,
    indexer: Option<Indexer>,
    discard: Option<Arc<dyn Discard>>,
    listener: Arc<dyn LibraryListener>,
    /// Why there is no index, when there is none and there should have been.
    index_error: Option<String>,
    /// Why search cannot go by meaning, when it was asked to and cannot.
    meaning_error: Arc<Mutex<Option<String>>>,
    /// Whether the embedding model may be loaded and used: the AI switch, as
    /// the indexer sees it. Off until [`Runtime::use_model`] says otherwise.
    meaning_gate: Arc<AtomicBool>,
    /// The model [`Runtime::use_model`] named, loaded whenever the switch
    /// goes on.
    model_folder: Arc<Mutex<Option<PathBuf>>>,
    /// Whether this runtime already follows an AI switch.
    following: AtomicBool,
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
        let discard = options.discard.or_else(|| {
            state::set_aside_path(&root)
                .map(|folder| Arc::new(SetAside::new(folder)) as Arc<dyn Discard>)
        });

        // An index that will not open is a cache that will not open. The
        // library is what matters and it is already loaded, so the runtime
        // carries on without one and says why when asked.
        let meaning_error = Arc::new(Mutex::new(None));
        let meaning_gate = Arc::new(AtomicBool::new(false));
        let path = options.index.or_else(|| state::index_path(&root));
        let opened = path.map(|path| {
            open_index(
                &path,
                &core,
                &listener,
                Arc::clone(&meaning_gate),
                Arc::clone(&meaning_error),
            )
        });
        let (reader, indexer, index_error) = match opened {
            Some(Ok((reader, indexer))) => (Some(reader), Some(indexer), None),
            Some(Err(error)) => (None, None, Some(error.to_string())),
            None => (
                None,
                None,
                Some("this machine will not say where the user's home folder is".to_owned()),
            ),
        };

        // Copies that arrived while Aralo was not running. The indexer's first
        // sync may already be under way; it leaves the base of a snippet with
        // a copy waiting alone, so it cannot have moved the base from under
        // this merge.
        let merged = settle(&mut write(&core), reader.as_deref(), discard.as_deref());
        if !merged.is_empty() {
            if let Some(indexer) = &indexer {
                let _ = indexer.jobs.send(Job::Sync);
            }
        }

        let watch = if options.watch {
            let core = Arc::clone(&core);
            let listener = Arc::clone(&listener);
            let jobs = indexer.as_ref().map(Indexer::sender);
            let reader = reader.clone();
            let discard = discard.clone();
            Some(Watch::with_debounce(
                &root,
                writes,
                options.debounce,
                move |changes: Changes| {
                    let mut changes_to_tell = Vec::new();
                    {
                        let mut core = write(&core);
                        match core.reload() {
                            Ok(()) => {
                                let merged =
                                    settle(&mut core, reader.as_deref(), discard.as_deref());
                                if let Some(jobs) = &jobs {
                                    let _ = jobs.send(Job::Sync);
                                }
                                changes_to_tell.push(LibraryChange::Outside {
                                    paths: changes.paths,
                                });
                                if !merged.is_empty() {
                                    changes_to_tell.push(LibraryChange::Merged { copies: merged });
                                }
                            }
                            Err(error) => changes_to_tell.push(LibraryChange::Failed {
                                message: error.to_string(),
                            }),
                        }
                    }
                    for change in changes_to_tell {
                        listener.changed(change, &read(&core));
                    }
                },
            )?)
        } else {
            None
        };

        let runtime = Self {
            _watch: watch,
            core,
            reader,
            indexer,
            discard,
            listener,
            index_error,
            meaning_error,
            meaning_gate,
            model_folder: Arc::new(Mutex::new(None)),
            following: AtomicBool::new(false),
        };
        if !merged.is_empty() {
            runtime.announce(LibraryChange::Merged { copies: merged });
        }
        Ok(runtime)
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

    /// Changes something about the core that is not in the folder — the clock,
    /// the locale dates are written in — so nothing is reindexed and no
    /// listener is told a snippet changed.
    pub fn configure<T>(&self, change: impl FnOnce(&mut Core) -> T) -> T {
        change(&mut write(&self.core))
    }

    /// Reads the folder again, as if something outside had changed it. A shell
    /// offers this as "reload", for a folder that arrived while Aralo was not
    /// looking.
    pub fn reload(&self) -> Result<(), CoreError> {
        let merged = {
            let mut core = write(&self.core);
            core.reload()?;
            settle(&mut core, self.reader.as_deref(), self.discard.as_deref())
        };
        self.reindex();
        self.announce(LibraryChange::Reloaded);
        if !merged.is_empty() {
            self.announce(LibraryChange::Merged { copies: merged });
        }
        Ok(())
    }

    /// Both sides of a conflict copy that did not merge, for a resolver to
    /// show. `copy` is relative to the library root.
    pub fn conflict(&self, copy: &Path) -> Result<ConflictSides, CoreError> {
        let base = |id| {
            self.reader
                .as_deref()
                .and_then(|index| lock(index).base(id).ok().flatten())
        };
        read(&self.core).conflict(copy, base)
    }

    /// Settles a conflict copy the way the user decided, as an edit: the index
    /// catches up and the listener hears [`LibraryChange::Edited`]. The copy
    /// goes where a merged one goes.
    pub fn resolve_conflict(&self, copy: &Path, resolution: Resolution) -> Result<(), CoreError> {
        let discard = self.discard.as_deref().ok_or(CoreError::NowhereToDiscard)?;
        self.edit(|core| core.resolve_conflict(copy, resolution, discard))
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

    /// Searches by meaning with the model in `folder` whenever AI is on, and
    /// follows the switch in `ai` from now on (plan 4.10).
    ///
    /// While AI is on, the model is loaded on the indexer thread and the
    /// library embedded there; until that is done, search goes by words.
    /// While it is off, the model is never loaded: turning the switch off
    /// stops search using it at once and drops it from memory, and turning
    /// it on loads it again. [`Runtime::meaning_error`] says why search is
    /// going by words.
    pub fn use_model(&self, folder: PathBuf, ai: &AiSettings) {
        let Some(indexer) = &self.indexer else {
            *lock(&self.meaning_error) =
                Some("search by meaning needs the index, and there is none".to_owned());
            return;
        };
        *lock(&self.model_folder) = Some(folder);
        let follow = Follow {
            gate: Arc::clone(&self.meaning_gate),
            jobs: Arc::downgrade(&indexer.jobs),
            core: Arc::downgrade(&self.core),
            folder: Arc::clone(&self.model_folder),
            meaning_error: Arc::clone(&self.meaning_error),
        };
        if !self.following.swap(true, Ordering::SeqCst) {
            let watcher = follow.clone();
            ai.watch_switches(move |switches| watcher.set(switches.enabled));
        }
        follow.set(ai.switches_in_force().enabled);
    }

    /// Why search is going by words alone although a model was named: the
    /// model would not load, or there is no index to keep its vectors in.
    pub fn meaning_error(&self) -> Option<String> {
        lock(&self.meaning_error).clone()
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
            .field("discard", &self.discard)
            .field("index_error", &self.index_error)
            .finish()
    }
}

/// Merges the conflict copies in the library that merge cleanly, and returns
/// the copies it merged. A failure is not reported as one: the copy it was
/// working on is still in the folder beside its original, reported as a
/// conflict copy, and the next read of the folder tries it again.
fn settle(
    core: &mut Core,
    reader: Option<&Mutex<Index>>,
    discard: Option<&dyn Discard>,
) -> Vec<PathBuf> {
    let Some(discard) = discard else {
        return Vec::new();
    };
    if core.conflicts().is_empty() {
        return Vec::new();
    }
    let base = |id| reader.and_then(|index| lock(index).base(id).ok().flatten());
    core.merge_conflicts(base, discard)
        .map(|report| report.merged)
        .unwrap_or_default()
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
    meaning_gate: Arc<AtomicBool>,
    meaning_error: Arc<Mutex<Option<String>>>,
) -> Result<(Arc<Mutex<Index>>, Indexer), CoreError> {
    let writer = Index::open(path)?;
    let reader = Index::open(path)?;
    let indexer = Indexer::start(
        writer,
        Arc::clone(core),
        Arc::clone(listener),
        meaning_gate,
        meaning_error,
    )?;
    Ok((Arc::new(Mutex::new(reader)), indexer))
}

/// The thread that owns the index's writing connection.
struct Indexer {
    /// Held here and nowhere else strongly: an AI switch the runtime follows
    /// holds it weakly, so dropping the runtime still ends the thread.
    jobs: Arc<Sender<Job>>,
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
    /// Load the embedding model in this folder and search by meaning with it,
    /// if AI is still on.
    Model(PathBuf),
    /// AI went off: drop the model and search by words.
    Unload,
    /// Reply on this channel once everything queued before it is done.
    Flush(Sender<()>),
}

impl Indexer {
    fn start(
        mut index: Index,
        core: Arc<RwLock<Core>>,
        listener: Arc<dyn LibraryListener>,
        meaning_gate: Arc<AtomicBool>,
        meaning_error: Arc<Mutex<Option<String>>>,
    ) -> Result<Self, CoreError> {
        let (jobs, queue) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("aralo-index".to_owned())
            .spawn({
                move || {
                    // The library on disk moved on while Aralo was not running,
                    // so the first thing the index does is catch up with it.
                    sync(&mut index, &core, &listener);
                    let mut embedding = Embedding {
                        embedder: None,
                        gate: meaning_gate,
                        error: meaning_error,
                    };
                    work(&mut index, &core, &queue, &listener, &mut embedding);
                }
            })
            .map_err(|source| CoreError::Thread { source })?;
        Ok(Self {
            jobs: Arc::new(jobs),
            thread: Some(thread),
        })
    }

    fn sender(&self) -> Sender<Job> {
        self.jobs.as_ref().clone()
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
        drop(std::mem::replace(&mut self.jobs, Arc::new(dead)));
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
    embedding: &mut Embedding,
) {
    while let Ok(job) = queue.recv() {
        match job {
            Job::Sync => {
                sync(index, core, listener);
                embedding.catch_up(index, core);
            }
            Job::Model(folder) => embedding.load(&folder, index, core),
            Job::Unload => embedding.unload(core),
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

/// What the indexer thread knows about search by meaning: the model, when one
/// is loaded, and the AI switch it answers to.
struct Embedding {
    embedder: Option<Embedder>,
    gate: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
}

impl Embedding {
    fn allowed(&self) -> bool {
        self.gate.load(Ordering::SeqCst)
    }

    /// Loads a model and embeds the library with it, or says why not. A
    /// model that will not load leaves search going by words, and the one
    /// before it, if there was one, is dropped: the shell asked for this one.
    /// Nothing is read while AI is off, however the job got here.
    fn load(&mut self, folder: &Path, index: &mut Index, core: &RwLock<Core>) {
        if !self.allowed() {
            self.unload(core);
            return;
        }
        match Embedder::open(folder, index) {
            Ok(mut loaded) => {
                loaded.catch_up(index, core, &self.gate);
                self.embedder = Some(loaded);
                *lock(&self.error) = None;
            }
            Err(error) => {
                self.embedder = None;
                read(core).set_meaning(None);
                *lock(&self.error) = Some(error);
            }
        }
    }

    /// Drops the model and what search had of it.
    fn unload(&mut self, core: &RwLock<Core>) {
        self.embedder = None;
        read(core).set_meaning(None);
        if !self.allowed() {
            *lock(&self.error) = Some(Refusal::Off.to_string());
        }
    }

    fn catch_up(&mut self, index: &mut Index, core: &RwLock<Core>) {
        if let Some(embedder) = &mut self.embedder {
            embedder.catch_up(index, core, &self.gate);
        }
    }
}

/// The AI switch, as a runtime follows it: what [`Runtime::use_model`]
/// registers with the AI settings. Everything it holds of the runtime is weak
/// or shared, so the settings outliving the runtime keeps nothing running.
#[derive(Clone)]
struct Follow {
    gate: Arc<AtomicBool>,
    jobs: Weak<Sender<Job>>,
    core: Weak<RwLock<Core>>,
    folder: Arc<Mutex<Option<PathBuf>>>,
    meaning_error: Arc<Mutex<Option<String>>>,
}

impl Follow {
    fn set(&self, enabled: bool) {
        self.gate.store(enabled, Ordering::SeqCst);
        if enabled {
            let folder = lock(&self.folder).clone();
            if let (Some(jobs), Some(folder)) = (self.jobs.upgrade(), folder) {
                let _ = jobs.send(Job::Model(folder));
            }
            return;
        }
        // Search stops using the model now, not when the indexer gets to it.
        if let Some(core) = self.core.upgrade() {
            read(&core).set_meaning(None);
        }
        *lock(&self.meaning_error) = Some(Refusal::Off.to_string());
        if let Some(jobs) = self.jobs.upgrade() {
            let _ = jobs.send(Job::Unload);
        }
    }
}

/// The embedding model on the indexer thread, and the vectors it has made.
struct Embedder {
    model: Arc<Model>,
    /// Every vector the index keeps for this model, by content hash, so a
    /// save does not read them all back.
    known: HashMap<String, Vec<f32>>,
    /// What search was last handed, so a sync that changed nothing a vector
    /// depends on does not hand it the same again.
    published: Option<Vec<(SnippetId, String)>>,
}

impl Embedder {
    fn open(folder: &Path, index: &Index) -> Result<Self, String> {
        let model = Model::open(folder).map_err(|error| error.to_string())?;
        let known = index
            .vectors(model.id())
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter_map(|(hash, bytes)| {
                let vector = aralo_embed::from_bytes(&bytes)?;
                (vector.len() == model.dimensions()).then_some((hash, vector))
            })
            .collect();
        Ok(Self {
            model: Arc::new(model),
            known,
            published: None,
        })
    }

    /// Embeds what has no vector yet, keeps it, forgets what the library no
    /// longer holds, and hands search the result. The library's lock is held
    /// to see what is there, not while embedding.
    fn catch_up(&mut self, index: &mut Index, core: &RwLock<Core>, gate: &AtomicBool) {
        let plan = Plan::of(read(core).library(), |hash| self.known.contains_key(hash));
        if plan.is_settled() && self.published.as_ref() == Some(plan.snippets()) {
            return;
        }
        let made = plan.embed(&self.model);
        if !made.is_empty() {
            let stored: Vec<(String, Vec<u8>)> = made
                .iter()
                .map(|(hash, vector)| (hash.clone(), aralo_embed::to_bytes(vector)))
                .collect();
            // A vector that is not stored is made again next time: the index
            // is a cache, and this is the cheap kind of miss.
            let _ = index.put_vectors(self.model.id(), &stored);
            self.known.extend(made);
        }
        // The first pass after a load also clears out what an earlier model
        // left; after that, only a snippet going leaves anything to prune.
        let keep = plan.hashes();
        if self.published.is_none() || self.known.len() > keep.len() {
            self.known.retain(|hash, _| keep.contains(hash));
            let _ = index.keep_vectors(self.model.id(), &keep);
        }
        let meaning = plan.assemble(Arc::clone(&self.model), &self.known);
        // The switch may have gone off while this was embedding; then the
        // unload queued behind this job drops the model.
        if !gate.load(Ordering::SeqCst) {
            return;
        }
        read(core).set_meaning(Some(Arc::new(meaning)));
        self.published = Some(plan.into_snippets());
    }
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
