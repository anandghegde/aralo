use std::path::Path;
use std::process::{Command, Output};

fn aralo(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aralo"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn init_then_type_expands_the_starter_snippets() {
    let folder = tempfile::tempdir().unwrap();
    let library = folder.path().join("lib");
    let library = library.to_str().unwrap();

    let init = aralo(&["init", library]);
    assert!(init.status.success(), "{init:?}");

    let typed = aralo(&["type", library, "Ty, omw ->>\\n"]);
    assert_eq!(stdout(&typed), "Thank you, on my way →\n\n");

    let list = stdout(&aralo(&["list", library]));
    assert!(list.contains(";shrug"), "{list}");
}

#[test]
fn validate_fails_only_for_broken_files() {
    let starter = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/starter");
    let ok = aralo(&["validate", starter.to_str().unwrap()]);
    assert!(ok.status.success(), "{}", stdout(&ok));
    assert!(stdout(&ok).contains("README.md: note:"));

    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("bad.md"), "---\nabbr: [x\n---\n").unwrap();
    let bad = aralo(&["validate", folder.path().to_str().unwrap()]);
    assert_eq!(bad.status.code(), Some(1), "{}", stdout(&bad));
    assert!(stdout(&bad).contains("bad.md: error:"));

    let missing = aralo(&["validate", folder.path().join("nope").to_str().unwrap()]);
    assert_eq!(missing.status.code(), Some(2));
}
