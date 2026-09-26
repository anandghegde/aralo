//! The diagnostics report: what a beta tester pastes into an issue instead of
//! the telemetry Aralo does not have (PRD P8, plan section 11).
//!
//! It says which versions are running, how the app is set up, what the
//! permissions are, how big the library is, and what the local counters
//! ([`crate::counters`]) have seen. It says nothing about what is in the
//! library or what was typed: no snippet body, abbreviation, label, group
//! name, file name or path, no context sent to a model, no answer from one,
//! no profile name, address or key, and no error message, since a message can
//! quote any of those. A test builds a library full of marked strings, runs
//! it, and checks that none of them comes out
//! (`crates/aralo-ffi/tests/diagnostics.rs`).
//!
//! The way that holds is that every line is a fixed label and a number, a
//! yes or no, or a word from a fixed list. The strings that come from outside
//! are few and named: the app and system versions the shell passes, the
//! locale tag, and the bundle IDs the counters kept; each is trimmed to the
//! characters such a thing is written in.
//!
//! The report is built in the core so that every shell prints the same one,
//! and so that the test that guards it runs without a window server.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use aralo_library::Issue;
use aralo_snippet::SnippetKind;

use crate::ai::{is_local, AiSettings};
use crate::counters::{Count, Tallies, LATENCY_BUCKETS_MS};
use crate::{Core, Runtime};

/// What only the shell knows. Everything is optional: a command line has no
/// tap and no permissions to report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellFacts {
    /// The app's name for itself and its version, for example
    /// `Aralo 0.5.0 (42)`.
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

/// A report being put together, one section at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticReport {
    sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Section {
    title: &'static str,
    lines: Vec<(String, String)>,
}

impl DiagnosticReport {
    /// A report that starts with the versions and what the shell said.
    pub fn new(shell: &ShellFacts) -> Self {
        let mut report = Self {
            sections: Vec::new(),
        };
        let versions = report.section("Versions");
        versions.push(("core".into(), env!("CARGO_PKG_VERSION").into()));
        if !shell.app_version.is_empty() {
            versions.push(("app".into(), version_text(&shell.app_version)));
        }
        if !shell.os_version.is_empty() {
            versions.push(("system".into(), version_text(&shell.os_version)));
        }
        versions.push((
            "platform".into(),
            format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        ));

        let lines = [
            ("accessibility", shell.accessibility.map(granted)),
            ("input monitoring", shell.input_monitoring.map(granted)),
            ("tap running", shell.tap_running.map(yes_no)),
            ("secure input on", shell.secure_input.map(yes_no)),
            ("paused", shell.paused.map(yes_no)),
            (
                "excluded apps",
                shell.excluded_apps.map(|count| count.to_string()),
            ),
        ];
        let state: Vec<_> = lines
            .into_iter()
            .filter_map(|(label, value)| Some((label.to_owned(), value?)))
            .collect();
        if !state.is_empty() {
            report.section("Permissions and state").extend(state);
        }
        report
    }

    /// How big the library is and what shape it is in: counts only.
    pub fn library(&mut self, core: &Core) -> &mut Self {
        let library = core.library();
        let snippets = library.snippets();
        let enabled = snippets.iter().filter(|s| s.settings.enabled).count();
        let abbreviations: usize = snippets.iter().map(|s| s.file.front.abbr.len()).sum();
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        for snippet in snippets {
            *kinds.entry(kind_name(snippet.file.front.kind)).or_default() += 1;
        }
        let mut issues: BTreeMap<&str, usize> = BTreeMap::new();
        for diagnostic in core.diagnostics() {
            *issues.entry(issue_name(&diagnostic.issue)).or_default() += 1;
        }

        let section = self.section("Library");
        section.push(("location".into(), location(library.root()).into()));
        section.push(("snippets".into(), snippets.len().to_string()));
        section.push(("enabled".into(), enabled.to_string()));
        section.push(("disabled".into(), (snippets.len() - enabled).to_string()));
        section.push(("groups".into(), library.groups().len().to_string()));
        section.push(("abbreviations".into(), abbreviations.to_string()));
        for (kind, count) in kinds {
            section.push((format!("type {kind}"), count.to_string()));
        }
        section.push((
            "conflict copies waiting".into(),
            library.conflicts().len().to_string(),
        ));
        for (issue, count) in issues {
            section.push((format!("files with {issue}"), count.to_string()));
        }
        section.push(("locale".into(), locale_text(core.locale())));
        self
    }

    /// The library, and the parts of the runtime around it: the index and
    /// search by meaning.
    pub fn runtime(&mut self, runtime: &Runtime) -> &mut Self {
        runtime.read(|core| {
            self.library(core);
        });
        let section = self.section("Index");
        section.push(("index".into(), yes_no(runtime.has_index())));
        // Whether search goes by meaning, not why not: the reason is a
        // message, and a message can name a path.
        let meaning = runtime.read(|core| core.meaning().is_some());
        section.push(("search by meaning".into(), yes_no(meaning)));
        section.push((
            "meaning problem".into(),
            yes_no(runtime.meaning_error().is_some()),
        ));
        self
    }

    /// The AI settings as shapes: the switches, and for each profile its
    /// adapter, whether its endpoint is on this machine, and whether it has a
    /// key. Never a name, an address, a header or a key.
    pub fn ai(&mut self, settings: &AiSettings) -> &mut Self {
        let switches = settings.switches_in_force();
        let profiles = settings.profiles();
        let section = self.section("AI");
        section.push(("enabled".into(), yes_no(switches.enabled)));
        section.push(("local only".into(), yes_no(switches.local_only)));
        match profiles {
            Ok(profiles) => {
                section.push(("profiles".into(), profiles.len().to_string()));
                for (number, saved) in profiles.iter().enumerate() {
                    let place = if is_local(&saved.profile.base_url) {
                        "local"
                    } else {
                        "remote"
                    };
                    let mut value = format!("{} {place}", saved.profile.adapter.as_str());
                    value.push_str(if saved.has_key() {
                        ", key saved"
                    } else {
                        ", no key"
                    });
                    if saved.is_default {
                        value.push_str(", default");
                    }
                    if saved.capabilities.is_some() {
                        value.push_str(", probed");
                    }
                    section.push((format!("profile {}", number + 1), value));
                }
            }
            Err(_) => section.push(("profiles".into(), "unreadable".into())),
        }
        self
    }

    /// What the local counters have seen.
    pub fn counters(&mut self, tallies: &Tallies) -> &mut Self {
        let section = self.section("Counters");
        section.push(("since".into(), tallies.since.clone()));
        for count in Count::ALL {
            section.push((count.as_str().into(), tallies.count(count).to_string()));
        }

        let section = self.section("Match to plan latency");
        let mut lower = 0;
        for (bucket, total) in tallies.latency.iter().enumerate() {
            let label = match LATENCY_BUCKETS_MS.get(bucket) {
                Some(upper) => format!("{lower} to {upper} ms"),
                None => format!("{lower} ms or more"),
            };
            if let Some(upper) = LATENCY_BUCKETS_MS.get(bucket) {
                lower = *upper;
            }
            section.push((label, total.to_string()));
        }

        if !tallies.apps.is_empty() {
            let section = self.section("Expansions and undos by app");
            for (app, tally) in &tallies.apps {
                section.push((
                    bundle_text(app),
                    format!("{} expanded, {} undone", tally.expansions, tally.undos),
                ));
            }
        }

        if !tallies.providers.is_empty() {
            let section = self.section("Model calls by provider");
            for (provider, tally) in &tallies.providers {
                let calls: Vec<String> = tally
                    .calls
                    .iter()
                    .map(|(used_for, n)| format!("{used_for} {n}"))
                    .collect();
                let errors: Vec<String> = tally
                    .errors
                    .iter()
                    .map(|(kind, n)| format!("{kind} {n}"))
                    .collect();
                let mut value = if calls.is_empty() {
                    "no calls".to_owned()
                } else {
                    calls.join(", ")
                };
                if !errors.is_empty() {
                    let _ = write!(value, "; errors: {}", errors.join(", "));
                }
                section.push((word_text(provider), value));
            }
        }

        let section = self.section("Recent errors");
        if tallies.errors.is_empty() {
            section.push(("none".into(), String::new()));
        }
        for error in &tallies.errors {
            section.push((word_text(&error.at), word_text(&error.kind)));
        }
        self
    }

    /// The report as plain text, one `label: value` line each, ready to paste
    /// into an issue.
    pub fn render(&self) -> String {
        let mut out = String::from("Aralo diagnostics\n");
        out.push_str(
            "Counts and settings only. No snippet, typed text, context, file name or key.\n",
        );
        for section in &self.sections {
            let _ = write!(out, "\n{}\n", section.title);
            for (label, value) in &section.lines {
                if value.is_empty() {
                    let _ = writeln!(out, "  {label}");
                } else {
                    let _ = writeln!(out, "  {label}: {value}");
                }
            }
        }
        out
    }

    fn section(&mut self, title: &'static str) -> &mut Vec<(String, String)> {
        if let Some(index) = self.sections.iter().position(|s| s.title == title) {
            // Asked twice, as `runtime` after `library`: the second time
            // replaces the first rather than printing the section twice.
            self.sections[index].lines.clear();
            return &mut self.sections[index].lines;
        }
        self.sections.push(Section {
            title,
            lines: Vec::new(),
        });
        let last = self.sections.len() - 1;
        &mut self.sections[last].lines
    }
}

impl std::fmt::Display for DiagnosticReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render())
    }
}

/// Which kind of place the library is in, from its path. The path itself is
/// never printed: it holds the user's name and whatever they called the
/// folder.
fn location(root: &Path) -> &'static str {
    let path = root.to_string_lossy();
    if path.contains("/Library/Mobile Documents/") {
        "iCloud Drive"
    } else if path.contains("/Library/CloudStorage/Dropbox") || path.contains("/Dropbox/") {
        "Dropbox"
    } else if path.contains("/Library/CloudStorage/GoogleDrive") {
        "Google Drive"
    } else if path.contains("/Library/CloudStorage/OneDrive") {
        "OneDrive"
    } else if path.contains("/Library/CloudStorage/") {
        "another cloud folder"
    } else if std::env::var_os("HOME")
        .is_some_and(|home| !home.is_empty() && root.starts_with(home))
    {
        "home folder"
    } else {
        "elsewhere"
    }
}

fn kind_name(kind: SnippetKind) -> &'static str {
    match kind {
        SnippetKind::Text => "text",
        SnippetKind::Rich => "rich",
        SnippetKind::Command => "command",
        SnippetKind::Prompt => "prompt",
        SnippetKind::Script => "script",
    }
}

/// An issue's kind. What an issue carries — the reason a file did not parse,
/// the abbreviation that was refused — stays out.
fn issue_name(issue: &Issue) -> &'static str {
    match issue {
        Issue::Unreadable(_) => "read errors",
        Issue::Invalid(_) => "parse errors",
        Issue::NotASnippet => "no front matter",
        Issue::TooLarge { .. } => "too large",
        Issue::DuplicateId { .. } => "duplicate ids",
        Issue::ConflictCopy { .. } => "conflict copies",
        Issue::MissingId => "no id yet",
        Issue::UnsupportedKind(_) => "unsupported types",
        Issue::AbbreviationRejected { .. } => "refused abbreviations",
        Issue::SymlinkedFolder => "symlinked folders",
        Issue::NotDownloaded => "not downloaded",
        Issue::TooDeep => "folders too deep",
    }
}

fn yes_no(value: bool) -> String {
    if value { "yes" } else { "no" }.to_owned()
}

fn granted(value: bool) -> String {
    if value { "granted" } else { "not granted" }.to_owned()
}

/// Keeps what a version is written in: letters, digits, spaces, dots,
/// hyphens, plus signs and brackets, at most 64 of them.
fn version_text(text: &str) -> String {
    keep(text, 64, |c| {
        c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '-' | '+' | '(' | ')')
    })
}

/// A locale tag: `de_DE`, `ja-JP`, `en_US@calendar=gregorian`.
fn locale_text(text: &str) -> String {
    keep(text, 48, |c| {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@' | '=' | ';')
    })
}

/// A bundle ID, which the counters have already checked.
fn bundle_text(text: &str) -> String {
    keep(text, 128, |c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')
    })
}

/// A word the counters wrote: a kind, a provider, a minute.
fn word_text(text: &str) -> String {
    keep(text, 64, |c| {
        c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-' | ':' | '.')
    })
}

fn keep(text: &str, most: usize, allowed: impl Fn(char) -> bool) -> String {
    text.chars().filter(|c| allowed(*c)).take(most).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_keeps_only_what_a_version_is_written_in() {
        assert_eq!(version_text("Aralo 0.5.0 (42)"), "Aralo 0.5.0 (42)");
        assert_eq!(version_text("1.0\n<script>"), "1.0script");
        assert!(version_text(&"9".repeat(500)).len() <= 64);
    }

    #[test]
    fn a_library_location_is_a_kind_of_place() {
        assert_eq!(
            location(Path::new(
                "/Users/someone/Library/Mobile Documents/com~apple~CloudDocs/Aralo"
            )),
            "iCloud Drive"
        );
        assert_eq!(
            location(Path::new(
                "/Users/someone/Library/CloudStorage/Dropbox/Aralo"
            )),
            "Dropbox"
        );
        assert_eq!(location(Path::new("/Volumes/Work/Aralo")), "elsewhere");
    }
}
