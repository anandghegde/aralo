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

const ONE_GROUP: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>groupName</key><string>Work</string>
  <key>snippetsTE2</key>
  <array>
    <dict>
      <key>abbreviation</key><string>;sig</string>
      <key>label</key><string>Signature</string>
      <key>plainText</key><string>Best regards</string>
    </dict>
    <dict>
      <key>abbreviation</key><string>;wait</string>
      <key>plainText</key><string>one%delay:500%two</string>
    </dict>
  </array>
</dict>
</plist>"#;

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

#[test]
fn import_then_expand_then_export_round_trips() {
    let folder = tempfile::tempdir().unwrap();
    let source = folder.path().join("work.textexpander");
    std::fs::write(&source, ONE_GROUP).unwrap();
    let library = folder.path().join("lib");
    let library = library.to_str().unwrap();

    // One snippet carries a macro Aralo has no placeholder for, so the import
    // reports a snippet to look at and exits 1.
    let imported = aralo(&["import", source.to_str().unwrap(), library]);
    assert_eq!(imported.status.code(), Some(1), "{imported:?}");
    let report = stdout(&imported);
    assert!(report.contains("1 clean, 1 need an edit"), "{report}");
    assert!(report.contains("%delay"), "{report}");
    // A library filled from somewhere else does not also get the starter set.
    assert!(!stdout(&aralo(&["list", library])).contains(";shrug"));

    let expanded = aralo(&["expand", library, ";sig"]);
    assert!(expanded.status.success(), "{expanded:?}");
    assert!(
        stdout(&expanded).starts_with("Best regards"),
        "{}",
        stdout(&expanded)
    );

    let out = folder.path().join("out.json");
    let exported = aralo(&["export", library, out.to_str().unwrap()]);
    assert!(exported.status.success(), "{exported:?}");
    assert!(stdout(&exported).contains("2 snippets as json"));

    // Aralo's own export imports back with nothing to convert and nothing to
    // edit: the bodies are already templates.
    let back = folder.path().join("back");
    let again = aralo(&["import", out.to_str().unwrap(), back.to_str().unwrap()]);
    assert!(again.status.success(), "{again:?}");
    assert!(stdout(&again).contains("2 clean"), "{}", stdout(&again));
    assert_eq!(
        stdout(&aralo(&["list", back.to_str().unwrap()])),
        stdout(&aralo(&["list", library]))
    );
}

#[test]
fn a_dry_run_writes_nothing_and_json_is_the_same_report() {
    let folder = tempfile::tempdir().unwrap();
    let source = folder.path().join("work.textexpander");
    std::fs::write(&source, ONE_GROUP).unwrap();
    let library = folder.path().join("lib");
    aralo(&["init", library.to_str().unwrap()]);
    let before = stdout(&aralo(&["list", library.to_str().unwrap()]));

    let dry = aralo(&[
        "import",
        source.to_str().unwrap(),
        library.to_str().unwrap(),
        "--dry-run",
        "--into",
        "Imported/TextExpander",
        "--report",
        "json",
    ]);
    assert_eq!(dry.status.code(), Some(1), "{dry:?}");
    let report: serde_json::Value = serde_json::from_str(&stdout(&dry)).unwrap();
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["entries"][0]["group"][0], "Imported");
    // A dry run says where each snippet would go; `dry_run` says it did not.
    assert_eq!(
        report["entries"][0]["path"],
        "Imported/TextExpander/Work/signature.md"
    );
    assert_eq!(before, stdout(&aralo(&["list", library.to_str().unwrap()])));
}

#[test]
fn export_needs_to_know_the_format_it_cannot_guess() {
    let folder = tempfile::tempdir().unwrap();
    let library = folder.path().join("lib");
    let library = library.to_str().unwrap();
    aralo(&["init", library]);

    let out = folder.path().join("out.bin");
    let guess = aralo(&["export", library, out.to_str().unwrap()]);
    assert_eq!(guess.status.code(), Some(2));
    assert!(!out.exists());

    let told = aralo(&["export", library, out.to_str().unwrap(), "--format", "csv"]);
    assert!(told.status.success(), "{told:?}");
    let csv = std::fs::read_to_string(&out).unwrap();
    assert!(csv.starts_with("abbr,label,group,tags,body"), "{csv}");

    // A group narrows it to the folder and what is inside it.
    let some = aralo(&[
        "export", library, "-", "--format", "json", "--group", "Symbols",
    ]);
    assert!(some.status.success(), "{some:?}");
    let document: serde_json::Value = serde_json::from_str(&stdout(&some)).unwrap();
    let snippets = document["snippets"].as_array().unwrap();
    assert!(!snippets.is_empty());
    assert!(snippets
        .iter()
        .all(|snippet| snippet["group"][0] == "Symbols"));
}

#[test]
fn search_finds_a_snippet_by_name_and_by_what_is_in_its_body() {
    let folder = tempfile::tempdir().unwrap();
    let library = folder.path().join("lib");
    let library = library.to_str().unwrap();
    aralo(&["init", library]);

    // A label, matched loosely: the starter library has "Best regards".
    let loose = aralo(&["search", library, "bregs"]);
    assert!(loose.status.success(), "{loose:?}");
    assert!(stdout(&loose).contains("Best regards"), "{loose:?}");

    // No query at all is the whole library.
    let all = stdout(&aralo(&["search", library]));
    assert_eq!(
        all.lines().count(),
        stdout(&aralo(&["list", library])).lines().count()
    );

    // Nothing found is exit 1, the way `expand` reports the same thing.
    let missing = aralo(&["search", library, "zzzznothing"]);
    assert_eq!(missing.status.code(), Some(1), "{missing:?}");
    assert!(stdout(&missing).is_empty());
}

#[test]
fn search_finds_a_macro_no_importer_could_convert() {
    let folder = tempfile::tempdir().unwrap();
    let library = folder.path().join("lib");
    let source = folder.path().join("one-group.textexpander");
    std::fs::write(&source, ONE_GROUP).unwrap();
    let library = library.to_str().unwrap();

    // The import leaves `%delay:500%` in the body as the text it was and says
    // so in its report (ADR-0013). This is the other half of that promise:
    // the library can be searched for every one of them.
    aralo(&["import", source.to_str().unwrap(), library]);
    let found = aralo(&["search", library, "%delay:"]);
    assert!(found.status.success(), "{found:?}");
    let line = stdout(&found);
    assert!(line.contains("body: one%delay:500%two"), "{line}");
}

#[test]
fn search_narrows_by_group_limit_and_the_enabled_flag() {
    let folder = tempfile::tempdir().unwrap();
    let library = folder.path().join("lib");
    let library = library.to_str().unwrap();
    aralo(&["init", library]);

    let symbols = stdout(&aralo(&["search", library, "--group", "symbols"]));
    assert!(!symbols.is_empty());
    assert!(
        symbols.lines().all(|line| line.contains("[Symbols/")),
        "{symbols}"
    );

    // The footer says when the limit left something out.
    let capped = stdout(&aralo(&["search", library, "--limit", "1"]));
    assert_eq!(capped.lines().count(), 2, "{capped}");
    assert!(capped.contains("raise --limit"), "{capped}");
}

/// `aralo` with its state in `state`, so the AI switch it reads is the
/// test's and never the machine's.
fn aralo_in(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_aralo"))
        .env("ARALO_STATE", state)
        .env_remove("ARALO_MODEL")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn search_with_a_model_finds_a_snippet_by_what_it_means_while_ai_is_on() {
    let folder = tempfile::tempdir().unwrap();
    let state = folder.path().join("state");
    let library = folder.path().join("lib");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(
        library.join("greeting.md"),
        "---\nlabel: Greeting\nabbr: \";gr\"\n---\nhello world\n",
    )
    .unwrap();
    let library = library.to_str().unwrap();
    // The tiny model's numbers are random, but it averages tokens: "world
    // hello" is the vector "hello world" is, and no word search finds it.
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/embed/tiny");
    let model = model.to_str().unwrap();

    let words = aralo_in(&state, &["search", library, "world hello"]);
    assert_eq!(words.status.code(), Some(1), "{words:?}");

    // AI is off until it is switched on, and so is the model (plan 4.10):
    // the search goes by words and says why.
    let off = aralo_in(
        &state,
        &["search", library, "world hello", "--model", model],
    );
    assert_eq!(off.status.code(), Some(1), "{off:?}");
    assert!(
        String::from_utf8_lossy(&off.stderr).contains("AI is switched off"),
        "{off:?}"
    );

    assert!(aralo_in(&state, &["ai", "on"]).status.success());
    let meaning = aralo_in(
        &state,
        &["search", library, "world hello", "--model", model],
    );
    assert!(meaning.status.success(), "{meaning:?}");
    assert!(
        stdout(&meaning).contains("meaning: hello world"),
        "{meaning:?}"
    );

    // ARALO_MODEL does what --model does.
    let from_environment = Command::new(env!("CARGO_BIN_EXE_aralo"))
        .args(["search", library, "world hello"])
        .env("ARALO_STATE", &state)
        .env("ARALO_MODEL", model)
        .output()
        .unwrap();
    assert!(from_environment.status.success(), "{from_environment:?}");

    // A folder that is not a model is an error that says what is missing.
    let broken = aralo_in(&state, &["search", library, "hello", "--model", library]);
    assert_eq!(broken.status.code(), Some(2), "{broken:?}");
    assert!(
        String::from_utf8_lossy(&broken.stderr).contains("tokenizer.json"),
        "{broken:?}"
    );
}

/// A model server on this machine that streams "one two three banana" to
/// every chat for `m`, refuses any other model, and lists `m`.
fn tiny_server() -> String {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}/v1", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                head.push_str(&line);
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let body = String::from_utf8_lossy(&body);
            let (status, kind, answer) = if head.starts_with("GET") {
                (
                    "200 OK",
                    "application/json",
                    r#"{"data":[{"id":"m"}]}"#.to_owned(),
                )
            } else if !body.contains(r#""model":"m""#) {
                (
                    "404 Not Found",
                    "application/json",
                    r#"{"error":{"message":"no such model"}}"#.to_owned(),
                )
            } else {
                let mut events = String::new();
                for piece in ["one", " two", " three", " banana"] {
                    events.push_str(&format!(
                        "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{piece}\"}}}}]}}\n\n"
                    ));
                }
                events.push_str(
                    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\
                     \"usage\":{\"prompt_tokens\":9,\"completion_tokens\":4}}\n\ndata: [DONE]\n\n",
                );
                ("200 OK", "text/event-stream", events)
            };
            let _ = write!(
                socket,
                "HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{answer}",
                answer.len()
            );
        }
    });
    address
}

#[test]
fn conformance_runs_against_a_profile_and_its_reports_make_the_table() {
    let folder = tempfile::tempdir().unwrap();
    let state = folder.path().join("state");
    let server = tiny_server();
    let added = aralo_in(
        &state,
        &["ai", "add", "Local", "--base-url", &server, "--model", "m"],
    );
    assert!(added.status.success(), "{added:?}");

    // Off, which is how AI starts: nothing is run.
    let refused = aralo_in(&state, &["conformance", "--profile", "Local"]);
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("AI is switched off"));

    assert!(aralo_in(&state, &["ai", "on"]).status.success());
    let ran = aralo_in(
        &state,
        &[
            "conformance",
            "--profile",
            "Local",
            "--report",
            "-",
            "--timeout",
            "10",
        ],
    );
    let summary = String::from_utf8_lossy(&ran.stderr);
    assert!(ran.status.success(), "{summary}");
    assert!(summary.contains("It conforms."), "{summary}");
    // Not a JSON object, so JSON output fails; it is not required.
    assert!(summary.contains("fail  json_output"), "{summary}");
    let report: serde_json::Value = serde_json::from_slice(&ran.stdout).unwrap();
    assert_eq!(report["passed"], true);
    assert_eq!(report["endpoint"]["key"], false);
    assert_eq!(report["endpoint"]["model"], "m");

    let reports = folder.path().join("reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(reports.join("local.json"), &ran.stdout).unwrap();
    let endpoints = folder.path().join("endpoints.toml");
    std::fs::write(
        &endpoints,
        "[[endpoint]]\nname = \"Nowhere\"\nbase_url = \"https://nowhere.example/v1\"\n\
         model = \"x\"\nruns = \"by hand\"\n",
    )
    .unwrap();
    let table = aralo(&[
        "conformance",
        "table",
        reports.to_str().unwrap(),
        "--endpoints",
        endpoints.to_str().unwrap(),
    ]);
    assert!(table.status.success(), "{table:?}");
    let table = stdout(&table);
    assert!(
        table.contains("| `m` | yes | pass | pass | pass | pass | pass | skip | pass | **fail** |"),
        "{table}"
    );
    assert!(
        table.contains("**json_output** failed (not required)"),
        "{table}"
    );
    assert!(
        table.contains("| Nowhere | `https://nowhere.example/v1` |"),
        "{table}"
    );
}

#[test]
fn diagnostics_prints_counts_and_nothing_from_the_library() {
    let folder = tempfile::tempdir().unwrap();
    let state = folder.path().join("state");
    let library = folder.path().join("zqx-library");
    std::fs::create_dir_all(library.join("zqx-group")).unwrap();
    std::fs::write(
        library.join("zqx-group/zqx-file.md"),
        "---\nlabel: zqx-label\nabbr: \";zqx\"\n---\nzqx-body\n",
    )
    .unwrap();

    let output = aralo_in(&state, &["diagnostics", library.to_str().unwrap()]);
    assert!(output.status.success(), "{output:?}");
    let report = stdout(&output);
    assert!(report.starts_with("Aralo diagnostics\n"), "{report}");
    assert!(report.contains("  snippets: 1\n"), "{report}");
    assert!(report.contains("  expansion.matched: 0\n"), "{report}");
    assert!(!report.to_lowercase().contains("zqx"), "{report}");
}
