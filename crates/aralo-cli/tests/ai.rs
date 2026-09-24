//! `aralo ai` with the state folder in a temporary directory. These stay off
//! the keychain: every profile here is saved without a key.

use std::path::Path;
use std::process::{Command, Output};

fn aralo(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aralo"))
        .env("ARALO_STATE", state)
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_profile_from_a_preset_is_saved_without_a_key_and_says_where_to_get_one() {
    let state = tempfile::tempdir().unwrap();
    let empty = aralo(state.path(), &["ai", "status"]);
    assert!(empty.status.success(), "{}", stderr(&empty));
    assert!(stdout(&empty).contains("AI: off"), "{}", stdout(&empty));

    let added = aralo(state.path(), &["ai", "add", "--preset", "groq"]);
    assert!(added.status.success(), "{}", stderr(&added));
    let text = stdout(&added);
    assert!(text.contains("Saved Groq"), "{text}");
    assert!(text.contains("https://console.groq.com/keys"), "{text}");

    let local = aralo(
        state.path(),
        &[
            "ai",
            "add",
            "Home server",
            "--base-url",
            "http://127.0.0.1:8080/v1",
            "--model",
            "qwen",
            "--header",
            "X-Title=Aralo",
            "--default",
        ],
    );
    assert!(local.status.success(), "{}", stderr(&local));

    let status = stdout(&aralo(state.path(), &["ai", "status"]));
    assert!(status.contains("  Groq (no key)"), "{status}");
    assert!(status.contains("* Home server (no key)"), "{status}");
    assert!(status.contains("X-Title: Aralo"), "{status}");

    let file = std::fs::read_to_string(state.path().join("profiles.toml")).unwrap();
    assert!(file.contains("default_profile = \"Home server\""), "{file}");

    let renamed = aralo(
        state.path(),
        &[
            "ai",
            "edit",
            "groq",
            "--rename",
            "Fast",
            "--model",
            "llama-3.3-70b-versatile",
        ],
    );
    assert!(renamed.status.success(), "{}", stderr(&renamed));
    let removed = aralo(state.path(), &["ai", "remove", "Fast"]);
    assert!(removed.status.success(), "{}", stderr(&removed));
    let status = stdout(&aralo(state.path(), &["ai", "status"]));
    assert!(!status.contains("Fast"), "{status}");
}

#[test]
fn nothing_is_sent_while_ai_is_off_and_local_only_refuses_a_remote_endpoint() {
    let state = tempfile::tempdir().unwrap();
    aralo(state.path(), &["ai", "add", "--preset", "openai"]);
    for args in [
        &["ai", "test"][..],
        &["ai", "probe", "OpenAI"],
        &["ai", "models"],
        &["ai", "detect"],
    ] {
        let output = aralo(state.path(), args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(
            stderr(&output).contains("AI is switched off"),
            "{}",
            stderr(&output)
        );
    }

    assert!(aralo(state.path(), &["ai", "on"]).status.success());
    assert!(aralo(state.path(), &["ai", "local-only", "on"])
        .status
        .success());
    let status = stdout(&aralo(state.path(), &["ai", "status"]));
    assert!(status.contains("AI: on, this machine only"), "{status}");
    let refused = aralo(state.path(), &["ai", "test"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(
        stderr(&refused).contains("api.openai.com"),
        "{}",
        stderr(&refused)
    );
}

#[test]
fn a_key_is_never_an_argument_and_never_goes_in_a_header() {
    let state = tempfile::tempdir().unwrap();
    let as_argument = aralo(
        state.path(),
        &["ai", "add", "--preset", "openai", "--key", "sk-abc"],
    );
    assert_eq!(as_argument.status.code(), Some(2));

    let in_a_header = aralo(
        state.path(),
        &[
            "ai",
            "add",
            "Proxy",
            "--base-url",
            "https://proxy.example.com/v1",
            "--model",
            "m",
            "--header",
            "X-Auth=sk-proj-0123456789abcdefghijklmnopqrstuv",
        ],
    );
    assert_eq!(in_a_header.status.code(), Some(2));
    let message = stderr(&in_a_header);
    assert!(!message.contains("0123456789abcdef"), "{message}");
    assert!(!state.path().join("profiles.toml").exists());

    let missing = aralo(
        state.path(),
        &[
            "ai",
            "add",
            "--preset",
            "openai",
            "--key-env",
            "ARALO_TEST_NO_SUCH_VAR",
        ],
    );
    assert_eq!(missing.status.code(), Some(2));
    assert!(
        stderr(&missing).contains("is not set"),
        "{}",
        stderr(&missing)
    );
}

#[test]
fn presets_list_where_to_make_a_key() {
    let state = tempfile::tempdir().unwrap();
    let text = stdout(&aralo(state.path(), &["ai", "presets"]));
    assert!(text.contains("openrouter"), "{text}");
    assert!(
        text.contains("key: https://openrouter.ai/settings/keys"),
        "{text}"
    );
    assert!(text.contains("no key needed"), "{text}");
}

#[test]
fn commands_list_the_built_in_ones_and_a_librarys_and_run_only_with_ai_on() {
    let state = tempfile::tempdir().unwrap();
    let listed = aralo(state.path(), &["ai", "command", "list"]);
    assert!(listed.status.success(), "{}", stderr(&listed));
    let text = stdout(&listed);
    assert!(
        text.contains("built-in  Fix spelling and grammar"),
        "{text}"
    );
    assert_eq!(text.lines().count(), 7, "{text}");

    let library = state.path().join("Aralo");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(
        library.join("pirate.md"),
        "---\nlabel: Pirate\ntype: command\n---\nSay it like a pirate.\n",
    )
    .unwrap();
    let library_arg = library.to_string_lossy().into_owned();
    let listed = aralo(
        state.path(),
        &["ai", "command", "list", "--library", &library_arg],
    );
    let text = stdout(&listed);
    assert!(
        text.lines().last().unwrap().ends_with("library   Pirate"),
        "{text}"
    );

    let mut running = Command::new(env!("CARGO_BIN_EXE_aralo"))
        .env("ARALO_STATE", state.path())
        .args(["ai", "command", "run", "make it shorter"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    running
        .stdin
        .take()
        .unwrap()
        .write_all(b"A long sentence.")
        .unwrap();
    let refused = running.wait_with_output().unwrap();
    assert_eq!(refused.status.code(), Some(2));
    assert!(stderr(&refused).contains("off"), "{}", stderr(&refused));
    assert!(stdout(&refused).is_empty());

    let unknown = aralo(state.path(), &["ai", "command", "run", "No such thing"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(
        stderr(&unknown).contains("no command called"),
        "{}",
        stderr(&unknown)
    );
}
