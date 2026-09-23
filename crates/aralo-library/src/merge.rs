//! Three-way merge of two versions of one snippet file ([ADR-0006]).
//!
//! The base is the version this machine last saw. The front matter merges key
//! by key and the body line by line: a side that left something as it was in
//! the base takes the other side's change, both sides making the same change is
//! that change, and both sides changing one key, or overlapping lines, is a
//! conflict for the user. Nothing is guessed.
//!
//! Without a base there is nothing to tell a change from what was always
//! there, so only what both sides agree on merges.
//!
//! [ADR-0006]: ../../../docs/adr/0006-files-as-source-of-truth.md

use aralo_snippet::{FrontMatter, SnippetFile};
use serde_yaml_ng::{Mapping, Value};

/// What a merge came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Merged {
    /// Everything both sides changed fits together.
    Clean(Box<SnippetFile>),
    /// Something was changed on both sides, differently.
    Conflicted(Clashes),
}

/// Where the two sides disagree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Clashes {
    /// Front-matter keys both sides changed, in the order the file writes them.
    pub keys: Vec<String>,
    /// Both sides changed the same lines of the body.
    pub body: bool,
}

/// Merges `ours` and `theirs`, two edits of `base`.
pub fn merge(base: Option<&SnippetFile>, ours: &SnippetFile, theirs: &SnippetFile) -> Merged {
    if ours == theirs {
        return Merged::Clean(Box::new(ours.clone()));
    }
    let mut clashes = Clashes::default();

    let front = merge_front(
        base.map(|base| &base.front),
        &ours.front,
        &theirs.front,
        &mut clashes.keys,
    );
    let body = merge_body(
        base.map(|base| base.body.as_str()),
        &ours.body,
        &theirs.body,
    );
    clashes.body = body.is_none();

    match (front, body) {
        (Some(front), Some(body)) if clashes.keys.is_empty() => {
            Merged::Clean(Box::new(SnippetFile { front, body }))
        }
        _ => Merged::Conflicted(clashes),
    }
}

/// Key by key. `None` when the merged keys do not make a front matter this
/// version reads, which can only happen when two edits that are each valid
/// combine into something that is not; that is a conflict too.
fn merge_front(
    base: Option<&FrontMatter>,
    ours: &FrontMatter,
    theirs: &FrontMatter,
    clashes: &mut Vec<String>,
) -> Option<FrontMatter> {
    let base = base.map(mapping).unwrap_or_default();
    let ours = mapping(ours);
    let theirs = mapping(theirs);

    // The keys in the order ours writes them, then any only theirs has.
    let mut keys: Vec<&Value> = ours.keys().collect();
    keys.extend(theirs.keys().filter(|key| !ours.contains_key(*key)));
    keys.extend(
        base.keys()
            .filter(|key| !ours.contains_key(*key) && !theirs.contains_key(*key)),
    );

    let mut merged = Mapping::new();
    for key in keys {
        let (b, o, t) = (base.get(key), ours.get(key), theirs.get(key));
        let pick = if o == t {
            o
        } else if o == b {
            t
        } else if t == b {
            o
        } else {
            clashes.push(key.as_str().unwrap_or_default().to_owned());
            continue;
        };
        if let Some(value) = pick {
            merged.insert(key.clone(), value.clone());
        }
    }
    serde_yaml_ng::from_value(Value::Mapping(merged)).ok()
}

/// A front matter as the keys it writes. Serialising what was parsed cannot
/// fail; if it ever did, an empty mapping makes every key look changed, which
/// is a conflict rather than a silent loss.
fn mapping(front: &FrontMatter) -> Mapping {
    match serde_yaml_ng::to_value(front) {
        Ok(Value::Mapping(mapping)) => mapping,
        _ => Mapping::new(),
    }
}

/// Line by line, or `None` when both sides changed the same lines.
fn merge_body(base: Option<&str>, ours: &str, theirs: &str) -> Option<String> {
    if ours == theirs {
        return Some(ours.to_owned());
    }
    let base = base?;
    if ours == base {
        return Some(theirs.to_owned());
    }
    if theirs == base {
        return Some(ours.to_owned());
    }
    // A body does not keep the newline that ends the file, and a line merge
    // treats a last line without one as a different line from the same text
    // with one. Every side gets one, and the merge gives it back.
    let merged = diffy::merge(
        &format!("{base}\n"),
        &format!("{ours}\n"),
        &format!("{theirs}\n"),
    )
    .ok()?;
    Some(merged.strip_suffix('\n').unwrap_or(&merged).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(text: &str) -> SnippetFile {
        SnippetFile::parse(text).unwrap()
    }

    fn clean(merged: Merged) -> SnippetFile {
        match merged {
            Merged::Clean(file) => *file,
            Merged::Conflicted(clashes) => panic!("conflicted: {clashes:?}"),
        }
    }

    const BASE: &str = "---\nlabel: Signature\nabbr: [;sig]\n---\nBest,\nSam\n\nSent from Aralo\n";

    #[test]
    fn changes_to_different_keys_and_lines_both_land() {
        let ours = file(
            "---\nlabel: Signature\nabbr: [;sig]\ncase: exact\n---\nBest,\nSam\n\nSent from Aralo\n",
        );
        let theirs = file(
            "---\nlabel: Email signature\nabbr: [;sig]\n---\nBest,\nSam\n\nSent from my Mac\n",
        );
        let merged = clean(merge(Some(&file(BASE)), &ours, &theirs));
        assert_eq!(merged.front.label, "Email signature");
        assert_eq!(merged.front.case, Some(aralo_snippet::CaseMode::Exact));
        assert_eq!(merged.body, "Best,\nSam\n\nSent from my Mac");
    }

    #[test]
    fn a_change_on_both_sides_to_one_key_is_a_conflict() {
        let ours = file("---\nlabel: Mine\nabbr: [;sig]\n---\nBest,\nSam\n\nSent from Aralo\n");
        let theirs = file("---\nlabel: Theirs\nabbr: [;sig]\n---\nBest,\nSam\n\nSent from Aralo\n");
        assert_eq!(
            merge(Some(&file(BASE)), &ours, &theirs),
            Merged::Conflicted(Clashes {
                keys: vec!["label".into()],
                body: false
            })
        );
    }

    #[test]
    fn overlapping_lines_are_a_conflict() {
        let ours =
            file("---\nlabel: Signature\nabbr: [;sig]\n---\nBest,\nSam L.\n\nSent from Aralo\n");
        let theirs =
            file("---\nlabel: Signature\nabbr: [;sig]\n---\nBest,\nSamuel\n\nSent from Aralo\n");
        assert_eq!(
            merge(Some(&file(BASE)), &ours, &theirs),
            Merged::Conflicted(Clashes {
                keys: vec![],
                body: true
            })
        );
    }

    #[test]
    fn a_key_removed_on_one_side_stays_removed() {
        let base = file("---\nlabel: X\nabbr: [x]\ntags: [a]\n---\nbody\n");
        let ours = file("---\nlabel: X\nabbr: [x]\n---\nbody\n");
        let theirs = file("---\nlabel: X\nabbr: [x]\ntags: [a]\n---\nnew body\n");
        let merged = clean(merge(Some(&base), &ours, &theirs));
        assert!(merged.front.tags.is_empty());
        assert_eq!(merged.body, "new body");
    }

    #[test]
    fn keys_this_version_does_not_know_merge_like_any_other() {
        let base = file("---\nabbr: [x]\nfuture: 1\n---\nbody\n");
        let ours = file("---\nabbr: [x]\nfuture: 2\n---\nbody\n");
        let theirs = file("---\nabbr: [x]\nfuture: 1\nlater: yes\n---\nbody\n");
        let merged = clean(merge(Some(&base), &ours, &theirs));
        assert_eq!(merged.front.extra.get("future"), Some(&Value::from(2)));
        assert!(merged.front.extra.contains_key("later"));
    }

    #[test]
    fn without_a_base_only_agreement_merges() {
        let ours = file("---\nabbr: [x]\n---\none\n");
        assert_eq!(clean(merge(None, &ours, &ours.clone())), ours);
        let theirs = file("---\nabbr: [x]\n---\ntwo\n");
        assert!(matches!(
            merge(None, &ours, &theirs),
            Merged::Conflicted(Clashes { body: true, .. })
        ));
    }

    #[test]
    fn the_last_line_merges_without_growing_a_newline() {
        let base = file("---\nabbr: [x]\n---\na\nb\nc\n");
        let ours = file("---\nabbr: [x]\n---\nA\nb\nc\n");
        let theirs = file("---\nabbr: [x]\n---\na\nb\nC\n");
        assert_eq!(clean(merge(Some(&base), &ours, &theirs)).body, "A\nb\nC");
    }
}
