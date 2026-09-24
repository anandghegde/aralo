//! The engine: buffer + snapshot + pause and scope state + one undo record.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use zeroize::Zeroize;

use crate::buffer::{RingBuffer, CAPACITY};
use crate::case::{detect_pattern, CasePattern};
use crate::presets::EXCLUDED_APP_PRESETS;
use crate::snapshot::{CaseMode, Hit, Pass, Snapshot, SnippetId, Trigger};

/// A key the shell reports. Everything that is not one of these (arrows,
/// shortcuts, mouse clicks, app switches) is reported through [`Engine::reset`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    /// A key that produced one character, after layout and dead-key translation.
    Char(char),
    Backspace,
    /// The platform's undo shortcut (Cmd+Z on macOS).
    Undo,
}

/// What the shell must do with the key it just reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyVerdict {
    /// Let the key through unchanged.
    Pass,
    /// An abbreviation was completed. Swallow the key if `consume`, send
    /// `delete_count` Backspaces, then insert the snippet.
    Match {
        snippet_id: SnippetId,
        /// Characters of the abbreviation that already reached the app.
        delete_count: u32,
        /// Always true today: the target app never sees the completing key.
        consume: bool,
        /// For adaptive-case snippets, how the expansion should be re-cased.
        case: CasePattern,
        /// The delimiter that triggered the match, to re-insert after the expansion.
        trailing: Option<char>,
    },
    /// The key undoes the expansion that just finished (PRD E6). Swallow it,
    /// remove the inserted text and type `retype` back.
    UndoLast {
        /// Backspaces that remove the inserted text, as the shell reported it.
        delete_count: u32,
        /// Exactly what the user had typed, including the delimiter.
        retype: String,
        /// How the text was inserted, so the shell can pick the undo technique.
        method: InsertMethod,
    },
}

/// Why a snippet the user picked from a list does not go in.
///
/// Neither reason is about the snippet: the same pick succeeds once Aralo is
/// resumed, or with another app in front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InsertRefusal {
    /// Aralo is paused. It inserts nothing anywhere until it is resumed, and a
    /// deliberate pick is no exception: pause means the keyboard is the user's.
    Paused,
    /// The app is one Aralo stays out of (PRD P2, E10). It expands nothing
    /// there however the expansion was asked for.
    ExcludedApp,
}

/// Why the shell is clearing the buffer. Every reason zeroes it (PRD P1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResetReason {
    MouseDown,
    /// Arrow keys, Home, End, Page Up and the like.
    Navigation,
    /// A key combination with Cmd or Ctrl.
    Shortcut,
    AppSwitch,
    FocusChange,
    SecureInput,
    /// An input method started composing (full IME support is E17, v1).
    InputMethod,
    /// A key event that did not map to exactly one character.
    UnmappableInput,
    Manual,
}

impl ResetReason {
    pub const ALL: [ResetReason; 9] = [
        ResetReason::MouseDown,
        ResetReason::Navigation,
        ResetReason::Shortcut,
        ResetReason::AppSwitch,
        ResetReason::FocusChange,
        ResetReason::SecureInput,
        ResetReason::InputMethod,
        ResetReason::UnmappableInput,
        ResetReason::Manual,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InsertMethod {
    Typed,
    Pasted,
}

/// The shell's report that it finished inserting an expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpansionRecord {
    pub snippet_id: SnippetId,
    /// Backspaces needed to remove what was inserted.
    pub delete_count: u32,
    pub method: InsertMethod,
}

/// What the user typed to trigger the last match, kept only until the next key.
struct TypedText {
    chars: [char; CAPACITY + 1],
    len: usize,
}

impl TypedText {
    fn as_string(&self) -> String {
        self.chars[..self.len].iter().collect()
    }
}

impl Drop for TypedText {
    fn drop(&mut self) {
        self.chars.zeroize();
    }
}

struct LastExpansion {
    snippet_id: SnippetId,
    typed: TypedText,
    /// Set by [`Engine::expansion_done`]. Until then an undo key is an ordinary key.
    done: Option<ExpansionRecord>,
}

/// The matcher state for one user session. Not thread-safe by design: the
/// bridge wraps it in a lock that is only ever held for one of these calls.
pub struct Engine {
    buffer: RingBuffer,
    snapshot: Arc<Snapshot>,
    front_app: String,
    paused: bool,
    /// Lower-case app IDs in which Aralo never records or expands (PRD P2, E10).
    excluded_apps: Vec<String>,
    front_app_excluded: bool,
    last: Option<LastExpansion>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// An engine with no abbreviations and the preset app exclusions.
    pub fn new() -> Self {
        let mut engine = Self {
            buffer: RingBuffer::new(),
            snapshot: Arc::new(Snapshot::empty()),
            front_app: String::new(),
            paused: false,
            excluded_apps: Vec::new(),
            front_app_excluded: false,
            last: None,
        };
        engine.set_excluded_apps(EXCLUDED_APP_PRESETS.iter().map(|&app| app.into()).collect());
        engine
    }

    /// Feeds one key. Runs on the event-tap thread and never allocates unless
    /// the verdict is [`KeyVerdict::UndoLast`].
    ///
    /// Rules, in order:
    ///
    /// 1. While paused, or while the front app is excluded, nothing is recorded.
    /// 2. Backspace or Undo directly after a finished expansion undoes it.
    /// 3. A delimiter completes the longest delimiter-triggered abbreviation
    ///    that ends at the buffer end; any character completes the longest
    ///    immediate abbreviation that ends with it. Longest wins, then the most
    ///    specific app scope, then immediate over delimiter.
    /// 4. Each candidate checks its own case mode, the whole-word rule and its
    ///    app scope before it counts.
    pub fn on_key(&mut self, event: KeyEvent) -> KeyVerdict {
        if self.is_suppressed() {
            self.clear();
            return KeyVerdict::Pass;
        }
        let last = self.last.take();
        match event {
            KeyEvent::Backspace | KeyEvent::Undo => {
                if let Some(LastExpansion {
                    typed,
                    done: Some(record),
                    ..
                }) = last
                {
                    self.buffer.clear();
                    return KeyVerdict::UndoLast {
                        delete_count: record.delete_count,
                        retype: typed.as_string(),
                        method: record.method,
                    };
                }
                if event == KeyEvent::Backspace {
                    self.buffer.pop();
                } else {
                    // An undo changes the document in a way the buffer cannot follow.
                    self.buffer.clear();
                }
                KeyVerdict::Pass
            }
            KeyEvent::Char(c) => self.on_char(c),
        }
    }

    fn on_char(&mut self, c: char) -> KeyVerdict {
        let by_delimiter = self
            .snapshot
            .find(&self.buffer, Pass::Delimiter(c), &self.front_app);
        self.buffer.push(c);
        let immediate = self
            .snapshot
            .find(&self.buffer, Pass::Immediate, &self.front_app);

        let rank = |hit: &Hit| (hit.depth, hit.specificity);
        let hit = match (immediate, by_delimiter) {
            (Some(now), Some(delim)) if rank(&delim) > rank(&now) => delim,
            (Some(now), _) => now,
            (None, Some(delim)) => delim,
            (None, None) => return KeyVerdict::Pass,
        };

        let entry = self.snapshot.entry(hit.entry);
        // The buffer now ends with `c`. An immediate match includes it; a
        // delimiter match sits just before it.
        let skip = usize::from(entry.trigger == Trigger::Delimiter);
        let mut typed = TypedText {
            chars: ['\0'; CAPACITY + 1],
            len: hit.depth + skip,
        };
        for i in 0..typed.len {
            typed.chars[i] = self.buffer.get_back(typed.len - 1 - i).unwrap_or('\0');
        }

        let case = match entry.case {
            CaseMode::Adaptive => detect_pattern(&typed.chars[..hit.depth], &entry.chars),
            CaseMode::Exact | CaseMode::Ignore => CasePattern::AsDefined,
        };
        let (delete_count, trailing) = match entry.trigger {
            // The completing character is swallowed, so it never reached the app.
            Trigger::Immediate => (hit.depth - 1, None),
            Trigger::Delimiter => (hit.depth, entry.keep_delimiter.then_some(c)),
        };
        let verdict = KeyVerdict::Match {
            snippet_id: entry.snippet_id,
            delete_count: delete_count as u32,
            consume: true,
            case,
            trailing,
        };

        self.last = Some(LastExpansion {
            snippet_id: entry.snippet_id,
            typed,
            done: None,
        });
        self.buffer.clear();
        verdict
    }

    /// Whether a command on selected text may run in `app_id`: read what is
    /// selected there, and paste over it.
    ///
    /// Not while paused, and never in an excluded app: a password manager's
    /// selection is not something to send anywhere. The buffer and the undo
    /// record are cleared, because the text around the caret is about to be
    /// replaced by something nobody typed.
    pub fn command_in(&mut self, app_id: &str) -> Result<(), InsertRefusal> {
        if self.paused {
            return Err(InsertRefusal::Paused);
        }
        if self.is_excluded(app_id) {
            return Err(InsertRefusal::ExcludedApp);
        }
        self.clear();
        Ok(())
    }

    /// Records a snippet the user picked from a list rather than typed, so
    /// that the undo key takes it back the way it takes any expansion back.
    /// [`Engine::expansion_done`] arms it, as it does for a match.
    ///
    /// `app_id` is the app the text is going into. It is named rather than read
    /// from [`Engine::set_front_app`] because a picker holds the keyboard while
    /// the user chooses: the front app is Aralo's own window, not the app the
    /// snippet is for.
    ///
    /// Nothing was typed to trigger it, so undo retypes nothing, and the buffer
    /// is cleared: whatever was typed before the picker opened no longer
    /// precedes the caret.
    pub fn chosen(&mut self, snippet_id: SnippetId, app_id: &str) -> Result<(), InsertRefusal> {
        if self.paused {
            return Err(InsertRefusal::Paused);
        }
        if self.is_excluded(app_id) {
            return Err(InsertRefusal::ExcludedApp);
        }
        // Naming the app the text is for is also naming the app that will have
        // the keyboard when it lands, so record it here. The shell reports the
        // same app a moment later, when the system says it came forward, and
        // that report then changes nothing: were it the first the engine heard
        // of it, it would clear the undo record between the insertion and the
        // user's undo key.
        self.set_front_app(app_id);
        self.clear();
        self.last = Some(LastExpansion {
            snippet_id,
            typed: TypedText {
                chars: ['\0'; CAPACITY + 1],
                len: 0,
            },
            done: None,
        });
        Ok(())
    }

    /// Arms undo for the match that was just reported. Ignored when another key
    /// arrived in between, so a late report can never undo the wrong thing.
    pub fn expansion_done(&mut self, record: ExpansionRecord) {
        if let Some(last) = &mut self.last {
            if last.snippet_id == record.snippet_id {
                last.done = Some(record);
            }
        }
    }

    /// Zeroes the buffer and forgets the undo record.
    pub fn reset(&mut self, _reason: ResetReason) {
        self.clear();
    }

    /// Sets the app that receives keys. A change of app resets the buffer.
    pub fn set_front_app(&mut self, app_id: &str) {
        if self.front_app.eq_ignore_ascii_case(app_id) {
            return;
        }
        self.front_app.clear();
        self.front_app.push_str(app_id);
        self.front_app_excluded = self.is_excluded(app_id);
        self.clear();
    }

    /// Global pause (PRD E10).
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.clear();
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Replaces the list of apps Aralo stays out of. Callers that want the
    /// presets include [`EXCLUDED_APP_PRESETS`] themselves.
    pub fn set_excluded_apps(&mut self, mut apps: Vec<String>) {
        for app in &mut apps {
            app.make_ascii_lowercase();
        }
        apps.sort();
        apps.dedup();
        self.excluded_apps = apps;
        self.front_app_excluded = self.is_excluded(&self.front_app);
        self.clear();
    }

    /// Swaps in a new snapshot and returns the old one, so the caller can drop
    /// a large trie away from the keystroke path.
    pub fn set_snapshot(&mut self, snapshot: Arc<Snapshot>) -> Arc<Snapshot> {
        self.clear();
        core::mem::replace(&mut self.snapshot, snapshot)
    }

    pub fn snapshot(&self) -> &Arc<Snapshot> {
        &self.snapshot
    }

    /// True when no typed character is held anywhere in the engine. Used by the
    /// privacy tests; carries no content.
    pub fn holds_no_keystrokes(&self) -> bool {
        self.buffer.is_zeroed() && self.last.is_none()
    }

    fn is_suppressed(&self) -> bool {
        self.paused || self.front_app_excluded
    }

    fn is_excluded(&self, app_id: &str) -> bool {
        self.excluded_apps
            .iter()
            .any(|app| app.eq_ignore_ascii_case(app_id))
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.last = None;
    }
}

// No derived `Debug`: the engine must not be able to print what was typed.
impl core::fmt::Debug for Engine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Engine")
            .field("abbreviations", &self.snapshot.len())
            .field("buffered", &self.buffer.len())
            .field("paused", &self.paused)
            .finish_non_exhaustive()
    }
}
