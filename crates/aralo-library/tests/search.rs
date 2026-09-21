//! Search over a real library folder: what ranks first, and what a hit says.

use std::fs;
use std::path::Path;

use aralo_library::{Field, Hit, Library, Query, Searcher};

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// A small library that has something to find in every field.
fn library(root: &Path) -> Library {
    write(
        root,
        "Work/best-regards.md",
        "---\nlabel: Best regards\nabbr: \";br\"\ntags: [email, sign-off]\n---\nBest regards,\nSam\n",
    );
    write(
        root,
        "Work/Billing/invoice.md",
        "---\nlabel: Invoice line\nabbr: \";inv\"\n---\nInvoice %key:tab% due on %d\n",
    );
    write(
        root,
        "Personal/thanks.md",
        "---\nlabel: Thanks\nabbr: [\";ty\", \";thx\"]\nenabled: false\n---\nThanks so much!\n",
    );
    write(
        root,
        "Personal/no-label.md",
        "---\nabbr: \";addr\"\n---\n12 Example Street\n",
    );
    Library::load(root).unwrap()
}

/// The labels a search returns, in the order it returned them.
fn names<'a>(library: &'a Library, hits: &[Hit]) -> Vec<&'a str> {
    hits.iter()
        .map(|hit| library.snippet(hit.id).unwrap().display_name())
        .collect()
}

fn search(library: &Library, query: &Query) -> Vec<Hit> {
    Searcher::new().search(library, query)
}

#[test]
fn an_empty_query_returns_every_snippet() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let hits = search(&library, &Query::default());
    assert_eq!(hits.len(), 4);
    assert!(hits.iter().all(|hit| hit.matched.is_empty()));
    // The library's own order, which is by path.
    assert_eq!(
        names(&library, &hits),
        [";addr", "Thanks", "Invoice line", "Best regards"]
    );
}

#[test]
fn the_abbreviation_you_typed_in_full_comes_first() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let hits = search(&library, &Query::new(";ty"));
    assert_eq!(hits[0].field, Field::Abbreviation);
    assert_eq!(hits[0].text, ";ty");
    assert_eq!(names(&library, &hits)[0], "Thanks");
}

#[test]
fn a_label_is_matched_fuzzily() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let hits = search(&library, &Query::new("bregs"));
    assert_eq!(names(&library, &hits), ["Best regards"]);
    assert_eq!(hits[0].field, Field::Label);
    assert_eq!(hits[0].text, "Best regards");
}

#[test]
fn the_characters_that_matched_come_back_for_highlighting() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let hits = search(&library, &Query::new("invoice"));
    let hit = &hits[0];
    let highlighted: String = hit
        .text
        .chars()
        .enumerate()
        .filter(|(index, _)| hit.matched.contains(&(*index as u32)))
        .map(|(_, character)| character)
        .collect();
    assert_eq!(highlighted.to_lowercase(), "invoice");
}

#[test]
fn a_snippet_with_no_label_is_listed_under_its_abbreviation() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    library(root);
    write(
        root,
        "nameless.md",
        "---\ntags: [draft]\n---\nNo name at all\n",
    );
    let library = Library::load(root).unwrap();

    // A hand-written file need not carry a label. The abbreviation names it,
    // and the first line of the body names one that has neither.
    let hits = search(&library, &Query::new("addr"));
    assert_eq!(hits[0].text, ";addr");
    let hits = search(&library, &Query::new("No name"));
    assert_eq!(hits[0].field, Field::Label);
    assert_eq!(hits[0].text, "No name at all");

    // The body is still searched, whatever the snippet is called.
    let hits = search(&library, &Query::new("Example Street"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].field, Field::Body);
    assert_eq!(hits[0].text, "12 Example Street");
}

/// ADR-0013 keeps a macro it cannot convert in the body as literal text,
/// because "a search of the library finds every one". This is that search.
#[test]
fn a_macro_left_in_a_body_is_found_by_searching_for_it() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let hits = search(&library, &Query::new("%key:tab%"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].field, Field::Body);
    assert_eq!(hits[0].text, "Invoice %key:tab% due on %d");
    assert_eq!(hits[0].matched, (8..17).collect::<Vec<u32>>());
}

#[test]
fn a_body_is_matched_literally_and_not_fuzzily() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    // Every one of these letters is in "Invoice %key:tab% due on %d", in
    // order. A fuzzy body search would call that a hit; a literal one does not.
    assert!(search(&library, &Query::new("ick%d")).is_empty());
}

#[test]
fn an_abbreviation_hit_outranks_a_body_hit() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    library(root);
    write(
        root,
        "mention.md",
        "---\nlabel: Mention\n---\nAsk ;br about it\n",
    );
    let library = Library::load(root).unwrap();

    let hits = search(&library, &Query::new(";br"));
    assert_eq!(hits[0].field, Field::Abbreviation);
    assert_eq!(hits[1].field, Field::Body);
    assert_eq!(names(&library, &hits), ["Best regards", "Mention"]);
}

#[test]
fn a_tag_and_a_group_are_searched_too() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());

    let hits = search(&library, &Query::new("sign-off"));
    assert_eq!(hits[0].field, Field::Tag);
    assert_eq!(hits[0].text, "sign-off");

    let hits = search(&library, &Query::new("Billing"));
    assert_eq!(hits[0].field, Field::Group);
    assert_eq!(hits[0].text, "Work/Billing");
}

#[test]
fn a_group_filter_keeps_the_group_and_what_is_inside_it() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let in_work = Query {
        group: Some(vec!["work".into()]),
        ..Query::default()
    };
    assert_eq!(
        names(&library, &search(&library, &in_work)),
        ["Invoice line", "Best regards"]
    );

    let in_billing = Query {
        group: Some(vec!["Work".into(), "Billing".into()]),
        ..Query::default()
    };
    assert_eq!(
        names(&library, &search(&library, &in_billing)),
        ["Invoice line"]
    );
}

#[test]
fn a_tag_filter_and_the_enabled_filter_narrow_the_search() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());

    let tagged = Query {
        tag: Some("EMAIL".into()),
        ..Query::default()
    };
    assert_eq!(
        names(&library, &search(&library, &tagged)),
        ["Best regards"]
    );

    let enabled = Query {
        enabled_only: true,
        ..Query::default()
    };
    let names = names(&library, &search(&library, &enabled));
    assert!(!names.contains(&"Thanks"), "{names:?}");
    assert_eq!(names.len(), 3);
}

#[test]
fn a_limit_cuts_the_list_after_ranking() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let hits = search(
        &library,
        &Query {
            limit: Some(2),
            ..Query::default()
        },
    );
    assert_eq!(names(&library, &hits), [";addr", "Thanks"]);
}

#[test]
fn a_query_that_matches_nothing_returns_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    assert!(search(&library, &Query::new("zzzzqqqq")).is_empty());
}

#[test]
fn one_snippet_is_reported_once_however_many_fields_match() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    // "Best regards" is the label, the body and near enough the abbreviation.
    let hits = search(&library, &Query::new("regards"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].field, Field::Label);
}
