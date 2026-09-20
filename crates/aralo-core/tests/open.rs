use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use aralo_core::engine::KeyEvent;
use aralo_core::{Core, Simulator, STARTER_FILES};

const APP: &str = "com.apple.TextEdit";

fn files_under(root: &Path) -> BTreeSet<String> {
    fn walk(root: &Path, folder: &Path, out: &mut BTreeSet<String>) {
        for entry in fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path.strip_prefix(root).unwrap();
                out.insert(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn the_embedded_starter_is_the_folder_in_the_repository() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/starter");
    let embedded: BTreeSet<String> = STARTER_FILES.iter().map(|(p, _)| (*p).to_owned()).collect();
    assert_eq!(embedded, files_under(&folder));
}

#[test]
fn a_new_library_gets_the_starter_and_expands_at_once() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().join("Aralo");
    let core = Core::open(&root).unwrap();
    assert_eq!(core.starter_files_written(), STARTER_FILES.len());
    assert!(root.join("aralo.yaml").exists());
    // The README is reported as not being a snippet; nothing else is wrong.
    assert_eq!(core.diagnostics().count(), 1);

    let mut field = Simulator::new(&core, APP);
    field.type_str("Hi, ty for the lift. Omw now ->> TY!");
    assert_eq!(
        field.text(),
        "Hi, thank you for the lift. On my way now → THANK YOU!"
    );
    assert_eq!(field.expansions(), 4);
}

#[test]
fn the_starter_is_written_once_and_never_over_the_users_files() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    Core::open(root).unwrap();
    fs::remove_file(root.join("Basics/thanks.md")).unwrap();
    fs::write(root.join("Basics/shrug.md"), "---\nabbr: ;shrug\n---\nmine").unwrap();

    let core = Core::open(root).unwrap();
    assert_eq!(core.starter_files_written(), 0);
    assert!(!root.join("Basics/thanks.md").exists());
    assert_eq!(
        Simulator::new(&core, APP).type_str(";shrug ").text(),
        "mine "
    );
}

#[test]
fn an_existing_folder_of_snippets_gets_a_manifest_and_nothing_else() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    fs::write(root.join("sig.md"), "---\nabbr: ;sig\n---\nBest,\nSam").unwrap();

    let core = Core::open(root).unwrap();
    assert_eq!(core.starter_files_written(), 0);
    assert_eq!(
        files_under(root),
        ["aralo.yaml", "sig.md"].map(String::from).into()
    );
    assert_eq!(core.snippets().len(), 1);
}

#[test]
fn open_read_only_writes_nothing() {
    let folder = tempfile::tempdir().unwrap();
    Core::open_read_only(folder.path()).unwrap();
    assert!(files_under(folder.path()).is_empty());
}

#[test]
fn undo_puts_back_exactly_what_was_typed() {
    let folder = tempfile::tempdir().unwrap();
    let core = Core::open(folder.path()).unwrap();
    let mut field = Simulator::new(&core, APP);
    field.type_str("so ;shrug").key(KeyEvent::Undo);
    assert_eq!(field.text(), "so ;shrug");

    // Undo is offered once, and only straight after the expansion.
    let mut field = Simulator::new(&core, APP);
    field.type_str("ty x").key(KeyEvent::Undo);
    assert_eq!(field.text(), "thank you x");
}

#[test]
fn return_and_tab_go_back_as_keys_and_are_not_undoable() {
    let folder = tempfile::tempdir().unwrap();
    let core = Core::open(folder.path()).unwrap();
    let mut field = Simulator::new(&core, APP);
    field.type_str("omw\n").key(KeyEvent::Undo);
    assert_eq!(field.text(), "on my way\n");
}

#[test]
fn reload_picks_up_edits_and_keeps_the_old_library_when_the_folder_goes() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().join("lib");
    let mut core = Core::open(&root).unwrap();
    fs::write(root.join("new.md"), "---\nabbr: ;new\n---\nfresh").unwrap();
    core.reload().unwrap();
    assert_eq!(
        Simulator::new(&core, APP).type_str(";new ").text(),
        "fresh "
    );

    fs::remove_dir_all(&root).unwrap();
    assert!(core.reload().is_err());
    assert_eq!(
        Simulator::new(&core, APP).type_str(";new ").text(),
        "fresh "
    );
}
