//! Local counters and the diagnostics report, over the bridge (plan 5.7).
//!
//! The core counts what it sees itself: matches, insertions, undos, resets,
//! imports, library trouble and model calls. What only the shell sees — the
//! event tap timing out, secure input coming on, a selection that could not be
//! read — it reports with [`Core::record_shell_event`]. The report is built in
//! the core from counts, settings and the facts the shell passes, and holds no
//! snippet, typed text, context, file name or key.

use std::sync::Arc;
use std::time::Instant;

use aralo_core::counters::Count;
use aralo_core::{DiagnosticReport, ShellFacts};

use crate::{AiProfiles, Core, Engine, ImportSummary, Shared};

/// Something that happened in the shell, for the counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ShellEvent {
    /// The system switched the event tap off because a callback took too long.
    TapTimedOut,
    /// The system switched the event tap off for user input, for example
    /// while secure input was on.
    TapDisabledByUser,
    /// The shell switched the event tap back on.
    TapReenabled,
    /// Accessibility or Input Monitoring was taken away while Aralo ran.
    PermissionRevoked,
    /// Secure input came on, so the tap sees no keys.
    SecureInputOn,
    /// A command could not read the selection in the app in front.
    SelectionUnreadable,
}

impl From<ShellEvent> for Count {
    fn from(event: ShellEvent) -> Self {
        match event {
            ShellEvent::TapTimedOut => Self::TapTimedOut,
            ShellEvent::TapDisabledByUser => Self::TapDisabledByUser,
            ShellEvent::TapReenabled => Self::TapReenabled,
            ShellEvent::PermissionRevoked => Self::PermissionRevoked,
            ShellEvent::SecureInputOn => Self::SecureInputOn,
            ShellEvent::SelectionUnreadable => Self::SelectionUnreadable,
        }
    }
}

/// What only the shell knows, for the report. A permission or state the
/// shell cannot tell is left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct DiagnosticFacts {
    /// For example `Aralo 0.5.0 (42)`.
    pub app_version: String,
    /// For example `macOS 15.1 (24B83)`.
    pub os_version: String,
    pub accessibility: Option<bool>,
    pub input_monitoring: Option<bool>,
    pub tap_running: Option<bool>,
    pub secure_input: Option<bool>,
    pub paused: Option<bool>,
    /// Apps the user excluded, not counting the built-in presets.
    pub excluded_apps: Option<u32>,
}

impl From<DiagnosticFacts> for ShellFacts {
    fn from(facts: DiagnosticFacts) -> Self {
        Self {
            app_version: facts.app_version,
            os_version: facts.os_version,
            accessibility: facts.accessibility,
            input_monitoring: facts.input_monitoring,
            tap_running: facts.tap_running,
            secure_input: facts.secure_input,
            paused: facts.paused,
            excluded_apps: facts.excluded_apps,
        }
    }
}

#[uniffi::export]
impl Core {
    /// Counts something only the shell saw. Cheap enough for the tap thread:
    /// one short lock, no file.
    pub fn record_shell_event(&self, event: ShellEvent) {
        self.shared.counters.add(event.into());
    }

    /// The diagnostics report, as text for the pasteboard: versions, the
    /// facts the shell passes, the library's size, the AI settings in outline
    /// when `ai` is given, and the local counters. It never holds a snippet,
    /// typed text, context, file name, profile name, address or key.
    ///
    /// The counters are saved as it is built, so a report that was copied
    /// matches the file.
    pub fn diagnostics_report(
        &self,
        facts: DiagnosticFacts,
        ai: Option<Arc<AiProfiles>>,
    ) -> String {
        let counters = &self.shared.counters;
        let _ = counters.save();
        let mut report = DiagnosticReport::new(&facts.into());
        report.runtime(&self.shared.runtime());
        if let Some(ai) = ai {
            report.ai(ai.settings());
        }
        report.counters(&counters.tallies());
        report.render()
    }

    /// Writes the counters to their file when a change has waited a minute.
    /// For a timer the shell already runs; calling it often costs nothing.
    pub fn save_counters_if_due(&self) {
        let _ = self.shared.counters.save_if_due();
    }

    /// Writes the counters now. For the app quitting.
    pub fn save_counters(&self) {
        let _ = self.shared.counters.save();
    }

    /// Starts every count again from nothing.
    pub fn clear_counters(&self) {
        self.shared.counters.clear();
    }
}

impl Engine {
    /// Counts a match from the keystroke path, and how long the plan took.
    pub(crate) fn count_match(&self, expand: &Option<aralo_core::Expand>, started: Instant) {
        let counters = &self.shared.counters;
        match expand {
            Some(aralo_core::Expand::Ready(_)) => {
                counters.latency(started.elapsed());
                counters.add(Count::Matched);
            }
            Some(aralo_core::Expand::Session(_)) => {
                counters.add(Count::Matched);
                counters.add(Count::SessionStarted);
            }
            None => counters.add(Count::RefusedSnippetGone),
        }
    }
}

impl Shared {
    /// Counts what an import did. A dry run did nothing.
    pub(crate) fn count_import(&self, summary: &ImportSummary) {
        if summary.dry_run {
            return;
        }
        let counters = &self.counters;
        counters.add(Count::ImportRuns);
        counters.add_n(
            Count::ImportedAsIs,
            u64::from(summary.imported.saturating_sub(summary.needs_edit)),
        );
        counters.add_n(Count::ImportedWithChanges, u64::from(summary.needs_edit));
        counters.add_n(Count::ImportSkipped, u64::from(summary.skipped));
    }
}
