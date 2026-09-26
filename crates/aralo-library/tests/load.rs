use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aralo_engine::{CaseMode, CasePattern, Engine, KeyEvent, KeyVerdict, Scope, Trigger};
use aralo_library::{Issue, Library};
use aralo_snippet::{FrontMatter, SnippetFile};

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn type_str(engine: &mut Engine, text: &str) -> Vec<KeyVerdict> {
    text.chars()
        .map(|c| engine.on_key(KeyEvent::Char(c)))
        .filter(|verdict| *verdict != KeyVerdict::Pass)
        .collect()
}

fn engine_for(library: &Library) -> Engine {
    let (snapshot, diagnostics) = library.snapshot();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let mut engine = Engine::new();
    engine.set_snapshot(Arc::new(snapshot));
    engine.set_front_app("com.apple.TextEdit");
    engine
}

fn issues(library: &Library) -> Vec<(String, &Issue)> {
    library
        .diagnostics()
        .iter()
        .map(|d| (d.path.to_string_lossy().replace('\\', "/"), &d.issue))
        .collect()
}

const ID_A: &str = "01J8ZK3V5Q8W6T9X2N4R7M0ABC";
const ID_B: &str = "01J8ZK3V5Q8W6T9X2N4R7M0ABD";

#[test]
fn settings_inherit_from_snippet_to_group_to_parent_to_built_in() {
    let folder = aralo_testkit::tempdir().unwrap();
    let root = folder.path();
    write(
        root,
        "Work/_group.yaml",
        "scope:\n  only: [com.apple.mail]\ndefaults:\n  trigger: immediate\n  delimiters: \" ;\"\n",
    );
    write(
        root,
        "Work/Billing/_group.yaml",
        "defaults:\n  case: exact\n",
    );
    write(
        root,
        "Work/Billing/late.md",
        &format!("---\nid: {ID_A}\nabbr: [;late]\nword: false\n---\nYour invoice is late."),
    );
    write(
        root,
        "top.md",
        &format!("---\nid: {ID_B}\nabbr: ty\n---\nthank you"),
    );

    let library = Library::load(root).unwrap();
    assert!(
        library.diagnostics().is_empty(),
        "{:?}",
        library.diagnostics()
    );

    let late = library.snippet(ID_A.parse().unwrap()).unwrap();
    assert_eq!(late.group, ["Work", "Billing"]);
    assert_eq!(late.path, PathBuf::from("Work/Billing/late.md"));
    assert_eq!(late.settings.trigger, Trigger::Immediate); // from Work
    assert_eq!(late.settings.case, CaseMode::Exact); // from Billing
    assert!(!late.settings.whole_word); // from the snippet
    assert!(late.settings.keep_delimiter); // built in
    assert_eq!(late.settings.delimiters, [' ', ';']); // from Work
    assert_eq!(
        late.settings.scope,
        Scope::Only(vec!["com.apple.mail".into()])
    );

    let top = library.snippet(ID_B.parse().unwrap()).unwrap();
    assert!(top.group.is_empty());
    assert_eq!(top.settings, aralo_library::Settings::default());
}

#[test]
fn a_disabled_group_switches_off_everything_beneath_it() {
    let folder = aralo_testkit::tempdir().unwrap();
    let root = folder.path();
    write(root, "Off/_group.yaml", "enabled: false\n");
    write(root, "Off/Inner/_group.yaml", "enabled: true\n");
    write(
        root,
        "Off/Inner/a.md",
        &format!("---\nid: {ID_A}\nabbr: aaa\n---\nA"),
    );
    write(
        root,
        "b.md",
        &format!("---\nid: {ID_B}\nabbr: bbb\nenabled: false\n---\nB"),
    );

    let library = Library::load(root).unwrap();
    assert_eq!(library.snippets().len(), 2);
    assert!(library.snippets().iter().all(|s| !s.settings.enabled));
    let (snapshot, _) = library.snapshot();
    assert!(snapshot.is_empty());
}

#[test]
fn one_bad_file_never_takes_the_library_down() {
    let folder = aralo_testkit::tempdir().unwrap();
    let root = folder.path();
    write(
        root,
        "good.md",
        &format!("---\nid: {ID_A}\nabbr: good\n---\nfine"),
    );
    write(root, "README.md", "# Notes about this folder\n");
    write(root, "broken.md", "---\nabbr: [unclosed\n---\nbody");
    write(
        root,
        "z-copy.md",
        &format!("---\nid: {ID_A}\nabbr: dup\n---\nsame id"),
    );
    write(root, "no-id.md", "---\nabbr: fresh\n---\nnew file");
    write(
        root,
        "script.md",
        &format!("---\nid: {ID_B}\nabbr: js\ntype: script\n---\n1+1"),
    );
    write(root, "Bad/_group.yaml", "defaults: [not, a, map]\n");
    write(
        root,
        "Bad/x.md",
        "---\nid: 01J8ZK3V5Q8W6T9X2N4R7M0ABE\nabbr: xx\n---\nX",
    );
    write(root, "notes.txt", "ignored");
    write(root, ".hidden/h.md", "---\nabbr: hidden\n---\nnever loaded");
    write(root, "_drafts/d.md", "---\nabbr: draft\n---\nnever loaded");
    write(
        root,
        "assets/logo.md",
        "---\nabbr: asset\n---\nnever loaded",
    );

    let library = Library::load(root).unwrap();
    let found = issues(&library);
    let has = |path: &str, test: fn(&Issue) -> bool| {
        found.iter().any(|(p, issue)| p == path && test(issue))
    };
    assert!(has("README.md", |i| *i == Issue::NotASnippet));
    assert!(has("broken.md", |i| matches!(i, Issue::Invalid(_))));
    assert!(has(
        "z-copy.md",
        |i| matches!(i, Issue::DuplicateId { first } if first == Path::new("good.md"))
    ));
    assert!(has("no-id.md", |i| *i == Issue::MissingId));
    assert!(has("script.md", |i| matches!(i, Issue::UnsupportedKind(_))));
    assert!(has("Bad/_group.yaml", |i| matches!(i, Issue::Invalid(_))));
    assert_eq!(found.len(), 6, "{found:?}");

    let mut labels: Vec<_> = library
        .snippets()
        .iter()
        .map(|s| s.path.to_string_lossy().replace('\\', "/"))
        .collect();
    labels.sort();
    assert_eq!(labels, ["Bad/x.md", "good.md", "no-id.md", "script.md"]);

    // The script snippet is loaded but not matchable; the rest expand.
    let mut engine = engine_for(&library);
    assert_eq!(type_str(&mut engine, "js good fresh xx ").len(), 3);
}

#[test]
fn a_missing_folder_is_an_error_and_a_newer_format_is_refused() {
    let folder = aralo_testkit::tempdir().unwrap();
    assert!(Library::load(&folder.path().join("nope")).is_err());

    write(folder.path(), "aralo.yaml", "format: 9999\n");
    assert!(Library::load(folder.path()).is_err());
}

#[test]
fn create_writes_a_manifest_once_and_write_snippet_round_trips() {
    let folder = aralo_testkit::tempdir().unwrap();
    let root = folder.path().join("My Library");
    assert!(Library::create(&root, Some("Mine".into())).unwrap());
    let manifest = fs::read_to_string(root.join("aralo.yaml")).unwrap();
    assert!(!Library::create(&root, Some("Other".into())).unwrap());
    assert_eq!(
        fs::read_to_string(root.join("aralo.yaml")).unwrap(),
        manifest
    );

    let library = Library::load(&root).unwrap();
    assert_eq!(library.manifest().unwrap().name.as_deref(), Some("Mine"));
    let file = SnippetFile {
        front: FrontMatter {
            id: Some(ID_A.parse().unwrap()),
            label: "Address".into(),
            abbr: vec![";addr".into()],
            ..FrontMatter::default()
        },
        body: "1 Example Street\nSpringfield".into(),
    };
    library
        .write_snippet(Path::new("Personal/address.md"), &file)
        .unwrap();

    let library = Library::load(&root).unwrap();
    let loaded = library.snippet(ID_A.parse().unwrap()).unwrap();
    assert_eq!(loaded.file, file);
    assert_eq!(loaded.group, ["Personal"]);
}

#[test]
fn the_starter_library_loads_cleanly_and_expands() {
    let starter = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/starter");
    let library = Library::load(&starter).unwrap();
    assert_eq!(
        issues(&library),
        [("README.md".to_owned(), &Issue::NotASnippet)]
    );
    assert!(library.snippets().len() >= 9);

    let mut engine = engine_for(&library);
    let verdicts = type_str(&mut engine, "Ty ");
    let [KeyVerdict::Match {
        delete_count: 2,
        case: CasePattern::Title,
        trailing: Some(' '),
        ..
    }] = verdicts.as_slice()
    else {
        panic!("{verdicts:?}");
    };
    // Symbols are immediate and match inside a word.
    assert_eq!(type_str(&mut engine, "a->>").len(), 1);
}

#[test]
fn a_sync_clients_copy_is_paired_with_its_original_not_loaded_beside_it() {
    let folder = aralo_testkit::tempdir().unwrap();
    let root = folder.path();
    let snippet = |body: &str| format!("---\nid: {ID_A}\nabbr: [;sig]\n---\n{body}");
    write(root, "Work/sig.md", &snippet("ours"));
    // Sorts before the original, which must still keep the id.
    write(
        root,
        "Work/sig (conflicted copy 2026-09-23).md",
        &snippet("theirs"),
    );
    write(root, "Work/sig 2.md", &snippet("iCloud's"));
    // Same id, a name no client gives a copy: a duplicate, as before.
    write(root, "Work/signature.md", &snippet("mine on purpose"));
    // A copy's name in another folder is not a copy.
    write(
        root,
        "Home/sig 2.md",
        &format!("---\nid: {ID_B}\nabbr: [;home]\n---\nhome"),
    );

    let library = Library::load(root).unwrap();
    let kept: Vec<_> = library
        .snippets()
        .iter()
        .map(|s| s.path.to_string_lossy().replace('\\', "/"))
        .collect();
    assert_eq!(kept, ["Home/sig 2.md", "Work/sig.md"]);

    let copies: Vec<_> = library
        .conflicts()
        .iter()
        .map(|c| {
            (
                c.copy.to_string_lossy().replace('\\', "/"),
                c.original.to_string_lossy().replace('\\', "/"),
            )
        })
        .collect();
    assert_eq!(
        copies,
        [
            (
                "Work/sig (conflicted copy 2026-09-23).md".to_owned(),
                "Work/sig.md".to_owned()
            ),
            ("Work/sig 2.md".to_owned(), "Work/sig.md".to_owned()),
        ]
    );
    let found = issues(&library);
    assert!(found.iter().any(|(path, issue)| path == "Work/sig 2.md"
        && matches!(issue, Issue::ConflictCopy { original } if original == Path::new("Work/sig.md"))));
    assert!(found.iter().any(|(path, issue)| path == "Work/signature.md"
        && matches!(issue, Issue::DuplicateId { first } if first == Path::new("Work/sig.md"))));
    assert!(library.in_conflict(library.snippets()[1].id));
    assert!(!library.in_conflict(library.snippets()[0].id));
}
