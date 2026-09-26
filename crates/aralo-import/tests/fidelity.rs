//! The exit criterion, measured rather than claimed: import the corpus in
//! `fixtures/import/` and check that at least 95% of it converts with nothing
//! left for a human to edit.
//!
//! Every source has a golden file beside it, `<name>.expected.json`, holding
//! what the import reported and what ended up in the library. The goldens are
//! compared byte for byte, so a change in the converter shows up as a diff of
//! the text a user would have got. To take a deliberate change:
//!
//! ```text
//! ARALO_BLESS=1 cargo test -p aralo-import --test fidelity
//! ```
//!
//! What the corpus is, and what this number does and does not say, is in
//! `fixtures/import/README.md`. In short: the files are written from the
//! documented formats, so this measures the converter against the
//! specification, not against a library exported from a real installation.

// The point of the harness is the number it prints.
#![allow(clippy::print_stdout)]

use std::path::{Path, PathBuf};

use aralo_import::{export, ExportOptions, Format, ImportOptions, ImportReport, Outcome};
use aralo_library::Library;

/// The share of a corpus that must import with no edit. M2 asked for 90%, M3
/// for 95%; the number only ever goes up, because a converter that lost ground
/// lost it on somebody's library.
const FLOOR: f64 = 0.95;

#[test]
fn the_corpus_imports_at_ninety_five_percent_or_better() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/import");
    let sources = sources(&corpus);
    assert!(!sources.is_empty(), "no sources in {}", corpus.display());

    let bless = std::env::var_os("ARALO_BLESS").is_some();
    let mut clean = 0;
    let mut total = 0;

    for source in &sources {
        let (report, snippets) = import(source);
        clean += report.count(Outcome::Clean);
        total += report.total();

        let golden = source.with_extension("expected.json");
        let found = record(&report, snippets);
        if bless {
            std::fs::write(&golden, &found).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&golden).unwrap_or_else(|error| {
            panic!(
                "{}: {error}. Write it with ARALO_BLESS=1 cargo test -p aralo-import",
                golden.display()
            )
        });
        assert_eq!(
            expected,
            found,
            "{} imports differently now. Compare, and if the change is wanted, \
             write the golden with ARALO_BLESS=1 cargo test -p aralo-import",
            source.display()
        );

        println!(
            "  {:<32} {:>3} snippets, {:>3} clean",
            relative(&corpus, source),
            report.total(),
            report.count(Outcome::Clean)
        );
    }

    let fidelity = clean as f64 / total as f64;
    println!(
        "corpus: {clean} of {total} snippets import with no edit ({:.1}%)",
        fidelity * 100.0
    );
    assert!(
        fidelity >= FLOOR,
        "import fidelity is {:.1}%, below the {:.0}% the roadmap asks for",
        fidelity * 100.0,
        FLOOR * 100.0
    );
}

/// Imports one source into a library of its own and hands back the report and
/// the library as JSON.
fn import(source: &Path) -> (ImportReport, serde_json::Value) {
    let folder = aralo_testkit::tempdir().unwrap();
    let root = folder.path();
    Library::create(root, None).unwrap();
    let library = Library::load(root).unwrap();

    let report = aralo_import::import(source, &library, &ImportOptions::default())
        .unwrap_or_else(|error| panic!("{}: {error}", source.display()));

    let library = Library::load(root).unwrap();
    let bytes = export(
        &library,
        &ExportOptions {
            format: Format::Json,
            group: Vec::new(),
        },
    )
    .unwrap();
    let mut document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // A fresh id is minted for every import, so it is not a golden value.
    for snippet in document["snippets"].as_array_mut().unwrap() {
        snippet.as_object_mut().unwrap().remove("id");
    }
    (report, document)
}

/// The golden text: what the user was told, then what they got.
fn record(report: &ImportReport, library: serde_json::Value) -> String {
    let value = serde_json::json!({
        "clean": report.count(Outcome::Clean),
        "needs_edit": report.count(Outcome::NeedsEdit),
        "skipped": report.count(Outcome::Skipped),
        "fidelity": (report.fidelity() * 1000.0).round() / 1000.0,
        "notes": report.notes,
        "entries": report.entries,
        "library": library["snippets"],
    });
    format!("{}\n", serde_json::to_string_pretty(&value).unwrap())
}

/// Every file in the corpus that is a source rather than a golden or a README.
fn sources(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(root, &mut found);
    found.retain(|path| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        !name.ends_with(".expected.json") && !name.eq_ignore_ascii_case("README.md")
    });
    found.sort();
    found
}

fn walk(folder: &Path, found: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(folder).unwrap_or_else(|error| panic!("{}: {error}", folder.display()));
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, found);
        } else {
            found.push(path);
        }
    }
}

fn relative<'a>(root: &Path, path: &'a Path) -> std::borrow::Cow<'a, str> {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy()
}
