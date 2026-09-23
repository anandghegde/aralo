//! M3's exit criterion for the evaluator, measured rather than claimed: every
//! case in `fixtures/golden/` expands to the same bytes, under a stopped
//! clock, in three locales.
//!
//! Each fixture holds the inputs — a body, what the user answered, what the
//! shell fetched — and has a transcript beside it, `<name>.expected.txt`,
//! holding what came out. The transcripts are compared byte for byte, so a
//! change anywhere in the expansion path shows up as a diff of the text a user
//! would have got. To take a deliberate change:
//!
//! ```text
//! ARALO_BLESS=1 cargo test -p aralo-core --test golden
//! ```
//!
//! What each fixture is for is in `fixtures/golden/README.md`. Two things are
//! checked that the transcript cannot show: that the caret ends up where the
//! body asked (the field is written with `|` in it), and that the preview an
//! editor shows is the expansion it would get, byte for byte.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aralo_core::clock::FixedClock;
use aralo_core::snippet::SnippetId;
use aralo_core::template::Diagnostic;
use aralo_core::{
    Answers, CivilTime, ContextValues, Core, Draft, Expand, Expansion, ProblemLevel, SessionStep,
    Simulator,
};

/// The three locales the goldens run in: the default, one that writes its
/// months in another language, and one that does not write in Latin letters
/// at all.
const LOCALES: [&str; 3] = ["en_US", "de_DE", "ja_JP"];

/// The moment every golden expands at: Monday 9 March 2026, 14:05:07, an hour
/// east of UTC. Afternoon, so a twelve-hour clock has something to say; a
/// single-digit day and month, so the padding directives differ from the
/// plain ones.
fn moment() -> CivilTime {
    CivilTime::new(2026, 3, 9, 14, 5, 7).with_offset(60)
}

const APP: &str = "com.apple.TextEdit";

/// One fixture file.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    /// What this file is about. It goes at the top of the transcript, so a
    /// diff says what it was meant to be checking.
    about: String,
    case: Vec<Case>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    /// What the case is called, in the transcript and in a failure.
    name: String,
    /// The body of the snippet under test. It is labelled `Case`, so a body
    /// that nests `{{snippet: Case}}` reaches itself.
    body: String,
    /// The abbreviation to give it. Only a case about `typed` needs another.
    #[serde(default = "default_abbr")]
    abbr: String,
    /// What the user types, when the point is typing: the case is otherwise
    /// picked from the palette, which is the same expansion with nothing to
    /// delete and no delimiter to put back.
    #[serde(default)]
    typed: Option<String>,
    /// What the user fills the form in with. A field left out keeps its
    /// default.
    #[serde(default)]
    answers: BTreeMap<String, String>,
    /// What the shell fetched, for a body that asks for it.
    #[serde(default)]
    clipboard: Option<String>,
    /// Other snippets in the library, for a body that nests them.
    #[serde(default)]
    nested: Vec<Nested>,
    /// The user presses Escape instead of answering.
    #[serde(default)]
    cancel: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Nested {
    /// What `{{snippet: …}}` names it by.
    label: String,
    body: String,
}

fn default_abbr() -> String {
    ";x".to_owned()
}

#[test]
fn every_golden_expands_the_same_way_in_three_locales() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/golden");
    let fixtures = fixtures(&folder);
    assert!(!fixtures.is_empty(), "no fixtures in {}", folder.display());

    let bless = std::env::var_os("ARALO_BLESS").is_some();
    for path in &fixtures {
        let text = std::fs::read_to_string(path).unwrap();
        let fixture: Fixture =
            toml::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let found = transcript(path, &fixture);

        let golden = path.with_extension("expected.txt");
        if bless {
            std::fs::write(&golden, &found).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&golden).unwrap_or_else(|error| {
            panic!(
                "{}: {error}. Write it with ARALO_BLESS=1 cargo test -p aralo-core --test golden",
                golden.display()
            )
        });
        assert_eq!(
            expected,
            found,
            "{} expands differently now. Compare, and if the change is wanted, \
             bless it with ARALO_BLESS=1 cargo test -p aralo-core --test golden",
            path.display()
        );
    }
}

fn fixtures(folder: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
        .unwrap_or_else(|error| panic!("{}: {error}", folder.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect();
    found.sort();
    found
}

/// Runs every case in every locale and writes down what happened.
fn transcript(path: &Path, fixture: &Fixture) -> String {
    let name = path.file_name().unwrap().to_string_lossy();
    let mut out = String::with_capacity(4096);
    let _ = writeln!(
        out,
        "# {name}: {about}\n\
         # Written by cargo test -p aralo-core --test golden. Do not edit; bless it.\n\
         # Expanded at 2026-03-09 14:05:07 +01:00. | is the caret, [] a selection.",
        about = fixture.about
    );
    for case in &fixture.case {
        let _ = write!(out, "\n=== {}\n", case.name);
        for locale in LOCALES {
            let _ = write!(out, "--- {locale}\n{}", run(case, locale));
        }
    }
    out
}

/// One case in one locale: what the field holds afterwards, what an editor
/// would have previewed, and what Aralo had to say about the body.
fn run(case: &Case, locale: &str) -> String {
    let folder = tempfile::tempdir().unwrap();
    let mut core = Core::open_without_starter(&folder.path().join("Aralo")).unwrap();
    let id = core
        .create_snippet(&[], &draft("Case", &case.abbr, &case.body))
        .unwrap();
    for nested in &case.nested {
        core.create_snippet(&[], &draft(&nested.label, "", &nested.body))
            .unwrap();
    }
    let core = core
        .with_clock(Arc::new(FixedClock(moment())))
        .in_locale(locale);

    let mut field = Simulator::new(&core, APP);
    for (name, value) in &case.answers {
        field = field.with_answer(name, value);
    }
    if let Some(clipboard) = &case.clipboard {
        field = field.with_clipboard(clipboard);
    }
    if case.cancel {
        field = field.cancelling();
    }
    match &case.typed {
        Some(keys) => {
            field.type_str(keys);
        }
        None => {
            field.insert(id);
        }
    }

    // What an editor shows beside the body is the expansion it would get:
    // the defaults, no context, and the same clock (PRD L10).
    let preview = core.preview(id).expect("the snippet is in the library");
    assert_eq!(
        preview,
        defaults_only(&core, id).plan.inserted_text(),
        "{}, {locale}: the preview is not what an expansion would insert",
        case.name
    );

    let mut out = String::new();
    let _ = writeln!(out, "field: {}", escape(&field.marked()));
    let _ = writeln!(out, "preview: {}", escape(&preview));
    match field.last_expansion() {
        Some(expansion) => {
            let _ = writeln!(
                out,
                "undo: {}",
                match expansion.undo_delete_count {
                    Some(count) => format!("{count} backspaces"),
                    None => "not by backspace".to_owned(),
                }
            );
        }
        None => {
            let _ = writeln!(out, "undo: nothing was inserted");
        }
    }
    for diagnostic in field.diagnostics() {
        let _ = writeln!(out, "{}", note(diagnostic));
    }
    out
}

/// The expansion for a snippet nobody answered anything for: what the preview
/// claims, driven through the session the shell would drive.
fn defaults_only(core: &Core, id: SnippetId) -> Expansion {
    let mut session = match core.insert(id).expect("the snippet is in the library") {
        Expand::Ready(expansion) => return expansion,
        Expand::Session(session) => *session,
    };
    loop {
        match session.step() {
            SessionStep::Form(_) => session.submit_form(Answers::new()),
            SessionStep::Context(_) => session.provide_context(ContextValues::default()),
            SessionStep::Ready(expansion) => return expansion,
        };
    }
}

fn draft(label: &str, abbr: &str, body: &str) -> Draft {
    Draft {
        label: label.to_owned(),
        abbr: if abbr.is_empty() {
            Vec::new()
        } else {
            vec![abbr.to_owned()]
        },
        body: body.to_owned(),
        ..Draft::default()
    }
}

fn note(diagnostic: &Diagnostic) -> String {
    let level = match diagnostic.level() {
        ProblemLevel::Error => "error",
        ProblemLevel::Note => "note",
    };
    format!("{level}: {}", escape(&diagnostic.message()))
}

/// One line per thing that happened: quoted, so a space at the end of an
/// expansion can be seen and no editor can tidy it away, and with the line
/// breaks written out, so one expansion stays on one line.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
