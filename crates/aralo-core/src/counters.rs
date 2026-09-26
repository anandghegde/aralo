//! Local counters: how often things happened, kept on this machine and never
//! sent anywhere (PRD P8).
//!
//! The MVP has no telemetry upload, and no code that could do one. What it
//! has is this: counts of expansions, undos, buffer resets, tap trouble,
//! imports and model calls, a latency histogram and the kinds of the last few
//! errors, which the diagnostics report prints ([`crate::report`]) and a beta
//! tester pastes into an issue by hand.
//!
//! Content cannot get in by construction. Every name a count is filed under
//! comes from an enum in this file, so the only strings stored are ones
//! written here. The two exceptions are chosen to carry no content: the apps
//! expansions went into are bundle IDs, which the operating system assigns,
//! and anything that does not look like one is filed as `other`; and the
//! providers model calls went to are an adapter kind and whether the endpoint
//! is on this machine, never a profile name or an address. An error is kept
//! as its kind and the minute it happened, never its message, because a
//! message can quote a path, an abbreviation or an answer from a model.
//!
//! The counts live in one small JSON file beside the index. Losing it costs
//! nothing but the counts.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The file the counts are kept in, inside Aralo's state folder.
pub const COUNTERS_FILE: &str = "counters.json";

/// How many error kinds the report remembers.
const RECENT_ERRORS: usize = 20;

/// How many apps get a line of their own. The rest are added up as `other`,
/// so a machine with a long history does not grow the file without end.
const MOST_APPS: usize = 100;

/// How long a change may wait in memory before [`Counters::save_if_due`]
/// writes it.
const SAVE_EVERY: Duration = Duration::from_secs(60);

/// The file format. A file with another version is started again.
const FORMAT: u32 = 1;

/// Something that happened, counted by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Count {
    /// An abbreviation matched and expanded with nothing to ask.
    Matched,
    /// An abbreviation matched and its snippet asked something first.
    SessionStarted,
    /// A session was cancelled, from its panel or because the tap went down.
    SessionCancelled,
    /// A snippet was picked from a list and inserted.
    Picked,
    /// Text went in by typing, and undo was armed for it.
    InsertedTyped,
    /// Text went in by pasting, and undo was armed for it.
    InsertedPasted,
    /// An expansion was taken back with the undo key.
    Undone,
    /// A pick or a command was refused because Aralo is paused.
    RefusedPaused,
    /// A pick or a command was refused in an excluded app.
    RefusedExcludedApp,
    /// A pick named a snippet that had gone.
    RefusedSnippetGone,
    /// The buffer was cleared, by reason.
    ResetMouseDown,
    ResetNavigation,
    ResetShortcut,
    ResetAppSwitch,
    ResetFocusChange,
    ResetSecureInput,
    ResetInputMethod,
    ResetUnmappableInput,
    ResetManual,
    /// macOS switched the tap off because a callback was slow.
    TapTimedOut,
    /// macOS switched the tap off because of user input.
    TapDisabledByUser,
    /// The watchdog found the tap off and switched it back on.
    TapReenabled,
    /// The Accessibility grant was taken away while the tap was up.
    PermissionRevoked,
    /// A password field or another secure-input holder took the keyboard.
    SecureInputOn,
    /// A command was asked for and the selection could not be read.
    SelectionUnreadable,
    /// An import ran (not a dry run).
    ImportRuns,
    /// Snippets an import brought in, as written.
    ImportedAsIs,
    /// Snippets an import brought in with something changed or dropped.
    ImportedWithChanges,
    /// Snippets an import left out.
    ImportSkipped,
    /// Conflict copies that merged on their own.
    ConflictsMerged,
    /// The library was read again after a change outside Aralo.
    OutsideChanges,
}

impl Count {
    pub const ALL: [Self; 31] = [
        Self::Matched,
        Self::SessionStarted,
        Self::SessionCancelled,
        Self::Picked,
        Self::InsertedTyped,
        Self::InsertedPasted,
        Self::Undone,
        Self::RefusedPaused,
        Self::RefusedExcludedApp,
        Self::RefusedSnippetGone,
        Self::ResetMouseDown,
        Self::ResetNavigation,
        Self::ResetShortcut,
        Self::ResetAppSwitch,
        Self::ResetFocusChange,
        Self::ResetSecureInput,
        Self::ResetInputMethod,
        Self::ResetUnmappableInput,
        Self::ResetManual,
        Self::TapTimedOut,
        Self::TapDisabledByUser,
        Self::TapReenabled,
        Self::PermissionRevoked,
        Self::SecureInputOn,
        Self::SelectionUnreadable,
        Self::ImportRuns,
        Self::ImportedAsIs,
        Self::ImportedWithChanges,
        Self::ImportSkipped,
        Self::ConflictsMerged,
        Self::OutsideChanges,
    ];

    /// The name the count is stored and printed under.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Matched => "expansion.matched",
            Self::SessionStarted => "expansion.session_started",
            Self::SessionCancelled => "expansion.session_cancelled",
            Self::Picked => "expansion.picked",
            Self::InsertedTyped => "insert.typed",
            Self::InsertedPasted => "insert.pasted",
            Self::Undone => "insert.undone",
            Self::RefusedPaused => "refused.paused",
            Self::RefusedExcludedApp => "refused.excluded_app",
            Self::RefusedSnippetGone => "refused.snippet_gone",
            Self::ResetMouseDown => "reset.mouse_down",
            Self::ResetNavigation => "reset.navigation",
            Self::ResetShortcut => "reset.shortcut",
            Self::ResetAppSwitch => "reset.app_switch",
            Self::ResetFocusChange => "reset.focus_change",
            Self::ResetSecureInput => "reset.secure_input",
            Self::ResetInputMethod => "reset.input_method",
            Self::ResetUnmappableInput => "reset.unmappable_input",
            Self::ResetManual => "reset.manual",
            Self::TapTimedOut => "tap.timed_out",
            Self::TapDisabledByUser => "tap.disabled_by_user_input",
            Self::TapReenabled => "tap.reenabled",
            Self::PermissionRevoked => "tap.permission_revoked",
            Self::SecureInputOn => "secure_input.on",
            Self::SelectionUnreadable => "command.selection_unreadable",
            Self::ImportRuns => "import.runs",
            Self::ImportedAsIs => "import.converted",
            Self::ImportedWithChanges => "import.approximated",
            Self::ImportSkipped => "import.skipped",
            Self::ConflictsMerged => "sync.conflicts_merged",
            Self::OutsideChanges => "sync.outside_changes",
        }
    }
}

/// What went wrong, as a kind. The message is never kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorKind {
    /// The folder changed and reading it failed.
    LibraryReadFailed,
    /// The search index would not open.
    IndexUnavailable,
    /// The embedding model would not load.
    ModelUnavailable,
    /// A compatibility table override did not parse.
    CompatTableRejected,
    /// An import stopped before it finished.
    ImportFailed,
    /// The policy stopped a model call before anything was sent.
    AiRefused,
    /// A model call got no answer: no route, a reset, a timeout.
    AiNetwork,
    /// The endpoint answered with an error status.
    AiStatus,
    /// The endpoint's answer could not be read.
    AiProtocol,
    /// A model call failed some other way: the keychain, the settings file.
    AiOther,
}

impl ErrorKind {
    pub const ALL: [Self; 10] = [
        Self::LibraryReadFailed,
        Self::IndexUnavailable,
        Self::ModelUnavailable,
        Self::CompatTableRejected,
        Self::ImportFailed,
        Self::AiRefused,
        Self::AiNetwork,
        Self::AiStatus,
        Self::AiProtocol,
        Self::AiOther,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LibraryReadFailed => "library_read_failed",
            Self::IndexUnavailable => "index_unavailable",
            Self::ModelUnavailable => "model_unavailable",
            Self::CompatTableRejected => "compat_table_rejected",
            Self::ImportFailed => "import_failed",
            Self::AiRefused => "ai_refused",
            Self::AiNetwork => "ai_network",
            Self::AiStatus => "ai_status",
            Self::AiProtocol => "ai_protocol",
            Self::AiOther => "ai_other",
        }
    }

    /// The kind of a failed model call.
    pub fn of_ai(error: &crate::ai::AiSettingsError) -> Self {
        use crate::ai::{AiError, AiSettingsError};
        match error {
            AiSettingsError::Ai(AiError::Refused(_)) => Self::AiRefused,
            AiSettingsError::Ai(AiError::Network(_)) => Self::AiNetwork,
            AiSettingsError::Ai(AiError::Status { .. }) => Self::AiStatus,
            AiSettingsError::Ai(AiError::Protocol(_)) => Self::AiProtocol,
            _ => Self::AiOther,
        }
    }
}

/// Where a model call went: the adapter, and whether the endpoint is on this
/// machine. Never the profile's name or address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provider {
    pub adapter: crate::ai::AdapterKind,
    pub local: bool,
}

impl Provider {
    /// The provider of a saved profile.
    pub fn of(profile: &crate::ai::Profile) -> Self {
        Self {
            adapter: profile.adapter,
            local: crate::ai::is_local(&profile.base_url),
        }
    }

    fn key(self) -> String {
        let place = if self.local { "local" } else { "remote" };
        format!("{} {place}", self.adapter.as_str())
    }
}

/// Which AI feature made a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiUse {
    Command,
    Block,
    Authoring,
}

impl AiUse {
    fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Block => "block",
            Self::Authoring => "authoring",
        }
    }
}

/// The upper bounds of the latency buckets, in milliseconds. The last bucket
/// has none.
pub const LATENCY_BUCKETS_MS: [u64; 5] = [1, 5, 10, 30, 100];

/// Model calls to one provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTally {
    /// Calls that started, by feature.
    pub calls: BTreeMap<String, u64>,
    /// Calls that failed, by error kind.
    pub errors: BTreeMap<String, u64>,
}

/// Expansions and undos in one app.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppTally {
    pub expansions: u64,
    pub undos: u64,
}

/// An error kind, and the minute it happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentError {
    pub kind: String,
    /// UTC, to the minute: `2026-09-26T14:03Z`.
    pub at: String,
}

/// Everything counted, as it is stored and as the report reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tallies {
    pub format: u32,
    /// When counting started on this machine, UTC to the minute.
    pub since: String,
    pub counts: BTreeMap<String, u64>,
    pub apps: BTreeMap<String, AppTally>,
    pub providers: BTreeMap<String, ProviderTally>,
    /// Time from a key that completed an abbreviation to the plan for it, by
    /// bucket: under 1 ms, under 5, under 10, under 30, under 100, and more.
    pub latency: Vec<u64>,
    /// The kinds of the last errors, oldest first.
    pub errors: VecDeque<RecentError>,
}

impl Default for Tallies {
    fn default() -> Self {
        Self {
            format: FORMAT,
            since: minute_now(),
            counts: BTreeMap::new(),
            apps: BTreeMap::new(),
            providers: BTreeMap::new(),
            latency: vec![0; LATENCY_BUCKETS_MS.len() + 1],
            errors: VecDeque::new(),
        }
    }
}

impl Tallies {
    pub fn count(&self, count: Count) -> u64 {
        self.counts.get(count.as_str()).copied().unwrap_or(0)
    }
}

/// The counters, shared by every thread that counts. Each call holds one
/// short lock and touches no file, so the keystroke path may count.
#[derive(Debug)]
pub struct Counters {
    tallies: Mutex<Tallies>,
    path: Option<PathBuf>,
    /// When a change was first left unsaved, if one is waiting.
    dirty_since: Mutex<Option<Instant>>,
}

impl Counters {
    /// Counters that are never written anywhere.
    pub fn in_memory() -> Self {
        Self {
            tallies: Mutex::new(Tallies::default()),
            path: None,
            dirty_since: Mutex::new(None),
        }
    }

    /// Counters kept in `path`, starting from what the file holds. A file that
    /// is missing, unreadable or from another version starts the counts
    /// again: they are counts, not work.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let tallies = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Tallies>(&bytes).ok())
            .filter(|tallies| tallies.format == FORMAT)
            .map(|mut tallies| {
                tallies.latency.resize(LATENCY_BUCKETS_MS.len() + 1, 0);
                tallies
            })
            .unwrap_or_default();
        Self {
            tallies: Mutex::new(tallies),
            path: Some(path),
            dirty_since: Mutex::new(None),
        }
    }

    /// Counters in [`COUNTERS_FILE`] in the folder given, or in Aralo's state
    /// folder when none is; in memory when the machine will not say where
    /// that is.
    pub fn in_folder(folder: Option<&Path>) -> Self {
        match folder
            .map(Path::to_path_buf)
            .or_else(crate::state::state_folder)
        {
            Some(folder) => Self::open(folder.join(COUNTERS_FILE)),
            None => Self::in_memory(),
        }
    }

    /// The counters for the folder given, shared with every other caller in
    /// this process that names the same folder while any of them still holds
    /// them. The keystroke path and the AI settings count into one file this
    /// way instead of overwriting each other's.
    pub fn shared(folder: Option<&Path>) -> Arc<Self> {
        static OPEN: Mutex<BTreeMap<PathBuf, Weak<Counters>>> = Mutex::new(BTreeMap::new());
        let Some(path) = folder
            .map(Path::to_path_buf)
            .or_else(crate::state::state_folder)
            .map(|folder| folder.join(COUNTERS_FILE))
        else {
            return Arc::new(Self::in_memory());
        };
        let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
        open.retain(|_, counters| counters.strong_count() > 0);
        if let Some(counters) = open.get(&path).and_then(Weak::upgrade) {
            return counters;
        }
        let counters = Arc::new(Self::open(&path));
        open.insert(path, Arc::downgrade(&counters));
        counters
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn add(&self, count: Count) {
        self.add_n(count, 1);
    }

    pub fn add_n(&self, count: Count, n: u64) {
        if n == 0 {
            return;
        }
        self.change(|tallies| {
            *tallies.counts.entry(count.as_str().to_owned()).or_default() += n;
        });
    }

    /// An expansion went into the app with this bundle ID.
    pub fn expanded_in(&self, app: &str) {
        self.change(|tallies| app_entry(tallies, app).expansions += 1);
    }

    /// An expansion was undone in the app with this bundle ID.
    pub fn undone_in(&self, app: &str) {
        self.change(|tallies| app_entry(tallies, app).undos += 1);
    }

    /// A model call started.
    pub fn ai_call(&self, provider: Provider, used_for: AiUse) {
        self.change(|tallies| {
            let tally = tallies.providers.entry(provider.key()).or_default();
            *tally.calls.entry(used_for.as_str().to_owned()).or_default() += 1;
        });
    }

    /// A model call failed. `provider` is `None` when it failed before one
    /// was chosen, which is filed under `none`.
    pub fn ai_error(&self, provider: Option<Provider>, kind: ErrorKind) {
        self.change(|tallies| {
            let key = provider.map_or_else(|| "none".to_owned(), Provider::key);
            let tally = tallies.providers.entry(key).or_default();
            *tally.errors.entry(kind.as_str().to_owned()).or_default() += 1;
        });
        self.error(kind);
    }

    /// How long a match took to become a plan.
    pub fn latency(&self, took: Duration) {
        let millis = u64::try_from(took.as_millis()).unwrap_or(u64::MAX);
        let bucket = LATENCY_BUCKETS_MS
            .iter()
            .position(|bound| millis < *bound)
            .unwrap_or(LATENCY_BUCKETS_MS.len());
        self.change(|tallies| {
            if let Some(slot) = tallies.latency.get_mut(bucket) {
                *slot += 1;
            }
        });
    }

    /// Something went wrong. Only the kind is kept.
    pub fn error(&self, kind: ErrorKind) {
        self.change(|tallies| {
            tallies.errors.push_back(RecentError {
                kind: kind.as_str().to_owned(),
                at: minute_now(),
            });
            while tallies.errors.len() > RECENT_ERRORS {
                tallies.errors.pop_front();
            }
        });
    }

    /// A copy of everything counted so far.
    pub fn tallies(&self) -> Tallies {
        self.lock().clone()
    }

    /// Starts counting again from nothing.
    pub fn clear(&self) {
        self.change(|tallies| *tallies = Tallies::default());
    }

    /// Writes the counts to their file, when they have one and something
    /// changed. The file is replaced whole, so a crash leaves the old one.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let Some(_) = self.take_dirty() else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(&self.tallies()).map_err(std::io::Error::other)?;
        let result = write_whole(path, &bytes);
        if result.is_err() {
            // Try again next time.
            self.mark_dirty();
        }
        result
    }

    /// Saves when a change has waited a minute. For a shell's timer, which
    /// may call this as often as it likes.
    pub fn save_if_due(&self) -> std::io::Result<()> {
        let due = self
            .dirty_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some_and(|since| since.elapsed() >= SAVE_EVERY);
        if due {
            self.save()
        } else {
            Ok(())
        }
    }

    fn change(&self, change: impl FnOnce(&mut Tallies)) {
        change(&mut self.lock());
        self.mark_dirty();
    }

    fn mark_dirty(&self) {
        self.dirty_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(Instant::now);
    }

    fn take_dirty(&self) -> Option<Instant> {
        self.dirty_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    fn lock(&self) -> MutexGuard<'_, Tallies> {
        // Every change is a single insert or increment, so a holder that
        // panicked left the counts consistent.
        self.tallies.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Counters {
    fn drop(&mut self) {
        let _ = self.save();
    }
}

/// The line an app's counts go on. A bundle ID is reverse-DNS letters,
/// digits, dots, hyphens and underscores; anything else is not one and is
/// filed as `other`, so no text from anywhere else can become a key.
fn app_entry<'a>(tallies: &'a mut Tallies, app: &str) -> &'a mut AppTally {
    let looks_like_bundle_id = !app.is_empty()
        && app.len() <= 128
        && app.contains('.')
        && app
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    let key = if !looks_like_bundle_id
        || (!tallies.apps.contains_key(app) && tallies.apps.len() >= MOST_APPS)
    {
        "other"
    } else {
        app
    };
    tallies.apps.entry(key.to_owned()).or_default()
}

fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

fn minute_now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%MZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_count_has_its_own_name() {
        let names: std::collections::BTreeSet<_> = Count::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(names.len(), Count::ALL.len());
        let kinds: std::collections::BTreeSet<_> =
            ErrorKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds.len(), ErrorKind::ALL.len());
    }

    #[test]
    fn counts_add_up_and_survive_a_restart() {
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join(COUNTERS_FILE);
        {
            let counters = Counters::open(&path);
            counters.add(Count::Matched);
            counters.add(Count::Matched);
            counters.add_n(Count::ImportedAsIs, 7);
            counters.expanded_in("com.apple.TextEdit");
            counters.undone_in("com.apple.TextEdit");
            counters.latency(Duration::from_micros(300));
            counters.latency(Duration::from_millis(40));
            counters.error(ErrorKind::IndexUnavailable);
        }
        let counters = Counters::open(&path);
        let tallies = counters.tallies();
        assert_eq!(tallies.count(Count::Matched), 2);
        assert_eq!(tallies.count(Count::ImportedAsIs), 7);
        assert_eq!(
            tallies.apps["com.apple.TextEdit"],
            AppTally {
                expansions: 1,
                undos: 1
            }
        );
        assert_eq!(tallies.latency, vec![1, 0, 0, 0, 1, 0]);
        assert_eq!(tallies.errors.len(), 1);
        assert_eq!(tallies.errors[0].kind, "index_unavailable");
    }

    #[test]
    fn a_damaged_file_starts_the_counts_again() {
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join(COUNTERS_FILE);
        std::fs::write(&path, "{ not json").unwrap();
        let counters = Counters::open(&path);
        assert_eq!(counters.tallies().count(Count::Matched), 0);
        counters.add(Count::Matched);
        counters.save().unwrap();
        assert_eq!(Counters::open(&path).tallies().count(Count::Matched), 1);
    }

    #[test]
    fn only_a_bundle_id_gets_a_line_of_its_own() {
        let counters = Counters::in_memory();
        counters.expanded_in("com.example.Editor");
        counters.expanded_in("thanks for your order, see attached");
        counters.expanded_in("");
        let apps = counters.tallies().apps;
        assert_eq!(apps["com.example.Editor"].expansions, 1);
        assert_eq!(apps["other"].expansions, 2);
        assert_eq!(apps.len(), 2);
    }

    #[test]
    fn only_the_last_errors_are_kept() {
        let counters = Counters::in_memory();
        for _ in 0..(RECENT_ERRORS + 5) {
            counters.error(ErrorKind::AiNetwork);
        }
        assert_eq!(counters.tallies().errors.len(), RECENT_ERRORS);
    }

    #[test]
    fn nothing_is_written_until_something_changed() {
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join(COUNTERS_FILE);
        let counters = Counters::open(&path);
        counters.save().unwrap();
        assert!(!path.exists());
        counters.add(Count::Undone);
        counters.save_if_due().unwrap();
        assert!(!path.exists(), "a change waits a minute");
        counters.save().unwrap();
        assert!(path.exists());
    }

    #[test]
    fn one_folder_has_one_set_of_counters_while_anyone_holds_it() {
        let folder = aralo_testkit::tempdir().unwrap();
        let first = Counters::shared(Some(folder.path()));
        let second = Counters::shared(Some(folder.path()));
        assert!(Arc::ptr_eq(&first, &second));
        let other = aralo_testkit::tempdir().unwrap();
        assert!(!Arc::ptr_eq(&first, &Counters::shared(Some(other.path()))));

        first.add(Count::Matched);
        drop((first, second));
        // The last one out saved, and the next one in reads it.
        let again = Counters::shared(Some(folder.path()));
        assert_eq!(again.tallies().count(Count::Matched), 1);
    }
}
