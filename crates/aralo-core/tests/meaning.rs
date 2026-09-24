//! Search by meaning, end to end (PRD A4, task 4.8).
//!
//! The first tests use the tiny model in `fixtures/embed/tiny`, whose numbers
//! are random: they check the plumbing, not the meaning. A static model
//! averages its tokens, so "world hello" means exactly what "hello world"
//! means to it, while no literal search finds one in the other. That is enough
//! to see a snippet found by meaning and nothing else.
//!
//! The last ones load the real model, the one the app ships, and ask the
//! search library for things in words its snippets do not use. They need the
//! model on disk (`make model`); CI fetches it and sets ARALO_REQUIRE_MODEL so
//! they cannot quietly skip there. Nothing in them can reach the network:
//! aralo-embed has no networking crate anywhere in its dependencies, which
//! `scripts/check-deps.sh` holds it to.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aralo_core::{
    Core, Field, LibraryChange, LibraryListener, Model, Query, Runtime, RuntimeOptions, SearchHit,
    MIN_SIMILARITY,
};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tiny() -> PathBuf {
    repo().join("fixtures/embed/tiny")
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Two snippets whose bodies are one another's words reversed.
fn library(root: &Path) {
    write(
        root,
        "greeting.md",
        "---\nid: 01M4000000000000000000MN01\nlabel: Greeting\nabbr: \";gr\"\n---\nhello world\n",
    );
    write(
        root,
        "farewell.md",
        "---\nid: 01M4000000000000000000MN02\nlabel: Farewell\nabbr: \";fw\"\n---\ngoodbye moon\n",
    );
}

fn names(hits: &[SearchHit]) -> Vec<(&str, Field)> {
    hits.iter()
        .map(|hit| (hit.name.as_str(), hit.field))
        .collect()
}

#[test]
fn a_core_given_a_model_finds_a_snippet_by_what_it_means() {
    let folder = tempfile::tempdir().unwrap();
    library(folder.path());
    let core = Core::open_read_only(folder.path()).unwrap();
    assert!(core.search(&Query::new("world hello")).is_empty());

    core.use_model(Arc::new(Model::open(&tiny()).unwrap()));
    assert_eq!(core.meaning().unwrap().len(), 2);
    let hits = core.search(&Query::new("world hello"));
    assert_eq!(names(&hits)[0], ("Greeting", Field::Meaning));
    assert_eq!(hits[0].text, "hello world");

    // Words still come first, and a query too short to mean anything is
    // left to them.
    let hits = core.search(&Query::new("gr"));
    assert!(hits.iter().all(|hit| hit.field != Field::Meaning));
}

struct Quiet;

impl LibraryListener for Quiet {
    fn changed(&self, _: LibraryChange, _: &Core) {}
}

fn runtime(folder: &Path, model: Option<PathBuf>) -> Runtime {
    let core = Core::open_without_starter(&folder.join("Aralo")).unwrap();
    let options = RuntimeOptions {
        index: Some(folder.join("state/index.sqlite3")),
        watch: false,
        model,
        ..RuntimeOptions::default()
    };
    Runtime::with_core(core, options, Arc::new(Quiet)).unwrap()
}

fn stored_vectors(folder: &Path) -> usize {
    let index = aralo_library::Index::open(&folder.join("state/index.sqlite3")).unwrap();
    let model = Model::open(&tiny()).unwrap();
    index.vectors(model.id()).unwrap().len()
}

#[test]
fn a_runtime_embeds_on_its_indexer_and_keeps_the_vectors() {
    let folder = tempfile::tempdir().unwrap();
    library(&folder.path().join("Aralo"));
    let runtime = runtime(folder.path(), Some(tiny()));
    runtime.flush();
    assert_eq!(runtime.meaning_error(), None);
    let hits = runtime.search(&Query::new("world hello"));
    assert_eq!(names(&hits)[0], ("Greeting", Field::Meaning));
    assert_eq!(stored_vectors(folder.path()), 2);

    // An edit is embedded again, and the old content's vector goes.
    write(
        &folder.path().join("Aralo"),
        "farewell.md",
        "---\nid: 01M4000000000000000000MN02\nlabel: Farewell\nabbr: \";fw\"\n---\nsee you later\n",
    );
    runtime.reload().unwrap();
    runtime.flush();
    let hits = runtime.search(&Query::new("later you see"));
    assert_eq!(names(&hits)[0], ("Farewell", Field::Meaning));
    assert_eq!(stored_vectors(folder.path()), 2);

    // A snippet that goes takes its vector with it.
    fs::remove_file(folder.path().join("Aralo/greeting.md")).unwrap();
    runtime.reload().unwrap();
    runtime.flush();
    assert_eq!(stored_vectors(folder.path()), 1);
    assert!(runtime
        .search(&Query::new("world hello"))
        .iter()
        .all(|hit| hit.name != "Greeting"));
}

#[test]
fn a_model_can_be_given_to_a_runtime_that_is_already_running() {
    let folder = tempfile::tempdir().unwrap();
    library(&folder.path().join("Aralo"));
    let runtime = runtime(folder.path(), None);
    runtime.flush();
    assert!(runtime.search(&Query::new("world hello")).is_empty());

    runtime.use_model(tiny());
    runtime.flush();
    let hits = runtime.search(&Query::new("world hello"));
    assert_eq!(names(&hits)[0], ("Greeting", Field::Meaning));
}

#[test]
fn a_model_that_will_not_load_leaves_search_to_the_words() {
    let folder = tempfile::tempdir().unwrap();
    library(&folder.path().join("Aralo"));
    let runtime = runtime(folder.path(), Some(folder.path().join("no-such-model")));
    runtime.flush();
    let problem = runtime.meaning_error().unwrap();
    assert!(problem.contains("tokenizer.json"), "{problem}");
    assert_eq!(
        names(&runtime.search(&Query::new("greeting"))),
        [("Greeting", Field::Label)]
    );

    // A model that does load clears the problem.
    runtime.use_model(tiny());
    runtime.flush();
    assert_eq!(runtime.meaning_error(), None);
}

// The real model.

/// The model the app ships, or `None` when it is not on this machine.
fn real_model() -> Option<Arc<Model>> {
    let folder = std::env::var_os("ARALO_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("models/potion-base-8M"));
    if !folder.join(aralo_core::embed::TENSORS_FILE).exists() {
        assert!(
            std::env::var_os("ARALO_REQUIRE_MODEL").is_none_or(|value| value.is_empty()),
            "ARALO_REQUIRE_MODEL is set and there is no model at {}",
            folder.display()
        );
        eprintln!(
            "no embedding model at {}; `make model` fetches it. Skipping.",
            folder.display()
        );
        return None;
    }
    Some(Arc::new(Model::open(&folder).unwrap()))
}

fn search_library() -> Core {
    Core::open_read_only(&repo().join("fixtures/search/library")).unwrap()
}

/// What each query found by meaning, and every snippet's similarity to it,
/// best first. The table is printed, and goes into every failure, so a change
/// of model or of the bar shows what the model thought.
fn by_meaning(core: &Core, text: &str) -> (Vec<String>, String) {
    let meaning = core.meaning().unwrap();
    let model = meaning.model();
    let query = model.embed(text);
    let mut scores: Vec<(f32, &str)> = core
        .library()
        .snippets()
        .iter()
        .map(|snippet| {
            let vector = model.embed(&aralo_library::meaning_text(snippet));
            (cosine(&query, &vector), snippet.display_name())
        })
        .collect();
    scores.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut table = format!("{text:?} (offered from {MIN_SIMILARITY})\n");
    for (similarity, name) in &scores {
        table.push_str(&format!("  {similarity:.3}  {name}\n"));
    }
    eprint!("{table}");
    let found = core
        .search(&Query::new(text))
        .into_iter()
        .filter(|hit| hit.field == Field::Meaning)
        .map(|hit| hit.name)
        .collect();
    (found, table)
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (norm(a) * norm(b)).max(f32::MIN_POSITIVE)
}

#[test]
fn money_back_finds_the_refund_reply_with_nothing_leaving_the_machine() {
    let Some(model) = real_model() else { return };
    let core = search_library();
    // The words find nothing: the reply never says "money" or "back".
    assert!(core.search(&Query::new("money back")).is_empty());

    core.use_model(model);
    let (found, table) = by_meaning(&core, "money back");
    assert_eq!(
        found.first().map(String::as_str),
        Some("Refund issued"),
        "{found:?}\n{table}"
    );
}

#[test]
fn other_things_are_found_by_what_they_mean_too() {
    let Some(model) = real_model() else { return };
    let core = search_library();
    core.use_model(model);
    for (query, wanted) in [
        ("give the customer their money back", "Refund issued"),
        ("my package is late", "Shipping delay"),
        ("forgot my login", "Password reset"),
        ("notes from our call", "Meeting follow-up"),
        ("on holiday", "Out of office"),
    ] {
        let (found, table) = by_meaning(&core, query);
        assert_eq!(
            found.first().map(String::as_str),
            Some(wanted),
            "{found:?}\n{table}"
        );
    }
}
