//! Export then import loses nothing.
//!
//! A library goes out as JSON, YAML and CSV, comes back into an empty library,
//! and the two libraries must hold the same snippets: same ids, same
//! abbreviations, same settings, same bodies. Only the file names may differ,
//! because an import builds them from the label.
//!
//! What an export does *not* carry is the group's own `_group.yaml`: a
//! snippet keeps the keys written in its own file. `docs/format/import.md`
//! says so.

use aralo_import::{export, import_bytes, ExportOptions, Format, ImportOptions, SnippetRecord};
use aralo_library::Library;
use aralo_snippet::{CaseMode, SnippetId, SnippetKind, TriggerMode};

#[test]
fn every_format_comes_back_with_the_same_snippets() {
    let source = aralo_testkit::tempdir().unwrap();
    let library = filled(source.path());
    let before = snippets(&library);
    assert_eq!(before.len(), 6);

    for format in [Format::Json, Format::Yaml, Format::Csv] {
        let bytes = export(
            &library,
            &ExportOptions {
                format,
                group: Vec::new(),
            },
        )
        .unwrap();

        let folder = aralo_testkit::tempdir().unwrap();
        Library::create(folder.path(), None).unwrap();
        let empty = Library::load(folder.path()).unwrap();
        let report = import_bytes(&bytes, format, &empty, &ImportOptions::default()).unwrap();
        assert_eq!(report.fidelity(), 1.0, "{format}: {report}");

        let after = snippets(&Library::load(folder.path()).unwrap());
        assert_eq!(after, before, "{format} did not round trip");
    }
}

#[test]
fn a_group_exports_and_comes_back_on_its_own() {
    let source = aralo_testkit::tempdir().unwrap();
    let library = filled(source.path());
    let bytes = export(
        &library,
        &ExportOptions {
            format: Format::Json,
            group: vec!["Work".into()],
        },
    )
    .unwrap();

    let folder = aralo_testkit::tempdir().unwrap();
    Library::create(folder.path(), None).unwrap();
    let empty = Library::load(folder.path()).unwrap();
    import_bytes(&bytes, Format::Json, &empty, &ImportOptions::default()).unwrap();

    let groups: Vec<String> = snippets(&Library::load(folder.path()).unwrap())
        .iter()
        .map(|snippet| snippet["group"][0].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(groups.len(), 3);
    assert!(groups.iter().all(|group| group == "Work"), "{groups:?}");
}

/// The library's snippets as JSON, in id order, so two libraries compare
/// whatever their file names are.
fn snippets(library: &Library) -> Vec<serde_json::Value> {
    let bytes = export(library, &ExportOptions::default()).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut snippets = document["snippets"].as_array().unwrap().clone();
    snippets.sort_by_key(|snippet| snippet["id"].as_str().unwrap_or_default().to_owned());
    snippets
}

/// A library holding one of everything the interchange formats have to carry.
fn filled(root: &std::path::Path) -> Library {
    Library::create(root, None).unwrap();
    let library = Library::load(root).unwrap();

    let mut records = Vec::new();

    let mut plain = SnippetRecord::new("Best regards,\nSam");
    plain.label = "Signature".into();
    plain.abbr = vec![";br".into(), ";rgds".into()];
    plain.group = vec!["Work".into()];
    plain.tags = vec!["email".into(), "sign-off".into()];
    records.push(plain);

    let mut settings = SnippetRecord::new("Immediately, exactly, whole word.");
    settings.label = "Every setting set".into();
    settings.abbr = vec![";set".into()];
    settings.group = vec!["Work".into(), "Email".into()];
    settings.trigger = Some(TriggerMode::Immediate);
    settings.case = Some(CaseMode::Exact);
    settings.word = Some(true);
    settings.keep_delimiter = Some(false);
    settings.enabled = Some(false);
    records.push(settings);

    let mut template = SnippetRecord::new(
        "Hi {{field: Name | default: there}},\n\n{{cursor}}\n\n{{date: %Y-%m-%d}}",
    );
    template.label = "Placeholders".into();
    template.abbr = vec![";hi".into()];
    template.group = vec!["Work".into(), "Email".into()];
    records.push(template);

    // Braces that are text, not a placeholder: the escape has to survive.
    let mut escaped = SnippetRecord::new("Use \\{{ and }} in the template.");
    escaped.label = "Escaped braces".into();
    escaped.abbr = vec![";brace".into()];
    records.push(escaped);

    let mut unicode = SnippetRecord::new("¯\\_(ツ)_/¯ — ¥€$, 日本語, 🇬🇧");
    unicode.label = "Unicode".into();
    unicode.abbr = vec![";shrug".into()];
    unicode.group = vec!["Symbols".into()];
    records.push(unicode);

    // A kind the expander does not run yet is still a snippet to carry.
    let mut script = SnippetRecord::new("date +%Y-%m-%d");
    script.label = "A script".into();
    script.abbr = vec![";stamp".into()];
    script.kind = SnippetKind::Script;
    records.push(script);

    for (index, mut record) in records.into_iter().enumerate() {
        record.id = Some(SnippetId::generate());
        let mut path = std::path::PathBuf::new();
        for folder in &record.group {
            path.push(folder);
        }
        path.push(format!("snippet-{index}.md"));
        library
            .write_snippet(&path, &record.to_snippet_file())
            .unwrap();
    }
    Library::load(root).unwrap()
}
