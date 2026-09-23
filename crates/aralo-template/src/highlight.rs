//! What an editor draws over a body.
//!
//! The editor highlights from the parser's own spans and diagnostics, so what
//! it shows is what an expansion does (PRD L10). A shell asks for one
//! [`BodyOutline`] per keystroke and turns it into colours and underlines; it
//! reads no part of the grammar itself, so two shells cannot disagree about
//! where a placeholder starts or what is wrong with it.
//!
//! Ranges are in UTF-16 code units, counted from the start of the body. That
//! is what a text view counts in on macOS and on Windows, and what a
//! JavaScript string counts in, so no shell converts anything. The parser's
//! own spans stay byte ranges, which is what Rust indexes with.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::eval::{resolve, Snippets};
use crate::parse::Placeholder;

/// A range of a body in UTF-16 code units, half-open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BodyRange {
    pub start: u32,
    pub end: u32,
}

impl BodyRange {
    pub fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

/// Everything an editor shows about a body it is not expanding.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BodyOutline {
    /// Every `{{…}}` the parser read, in the order they appear.
    pub placeholders: Vec<BodyPlaceholder>,
    /// What the editor should say about the body, in the order the problems
    /// appear. A body with none of them expands as it reads.
    pub problems: Vec<BodyProblem>,
}

/// One `{{…}}` the parser read, for the editor to colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyPlaceholder {
    /// The name, already lower-case: the parser rejects any other.
    pub name: String,
    /// True when [`catalogue`] lists this name. A name it does not list still
    /// parses, so this marks a placeholder to point at, not a broken one.
    pub known: bool,
    /// False for a name Aralo does not expand yet, such as an AI block: it
    /// inserts as the text it is written as.
    pub evaluated: bool,
    pub range: BodyRange,
}

/// Something for the editor to show under, or beside, a range of the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyProblem {
    pub level: ProblemLevel,
    /// One sentence, in the core's words, so every shell says the same thing.
    pub message: String,
    pub range: BodyRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProblemLevel {
    /// The body does not read the way it was written: the range expands as
    /// literal text, or part of it is dropped.
    Error,
    /// The body reads, and something in it is still worth a second look.
    Note,
}

/// One of the placeholders Aralo knows: what a menu offers to insert, and what
/// an unknown name is compared against.
///
/// The list is the format's, not a shell's, so the Mac menu and a Windows one
/// offer the same placeholders in the same words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaceholderInfo {
    pub name: &'static str,
    /// One line for a menu item.
    pub summary: &'static str,
    /// The text to insert at the caret.
    pub insert: &'static str,
    /// What to select inside `insert` once it is in, in UTF-16 code units from
    /// the start of `insert`: the part the user replaces with their own words.
    /// Empty for a placeholder that takes nothing.
    pub select: BodyRange,
    /// False while the evaluator does not expand it yet, in which case it
    /// inserts as the text it is written as.
    pub evaluated: bool,
}

/// Every placeholder the format defines, in the order a menu offers them.
///
/// `select` covers the part of `insert` a user replaces, so inserting
/// `{{field: name}}` leaves `name` selected and the next keystroke names the
/// field. The ranges are checked against `insert` by a test, so a change to one
/// without the other cannot ship.
pub fn catalogue() -> &'static [PlaceholderInfo] {
    CATALOGUE
}

/// One row of [`CATALOGUE`]: a placeholder an expansion puts a value in.
const fn entry(
    name: &'static str,
    summary: &'static str,
    insert: &'static str,
    select: BodyRange,
) -> PlaceholderInfo {
    PlaceholderInfo {
        name,
        summary,
        insert,
        select,
        evaluated: true,
    }
}

/// A row for a placeholder the format defines and Aralo does not expand yet:
/// it stays as written, and an editor says so from `evaluated`.
const fn pending(
    name: &'static str,
    summary: &'static str,
    insert: &'static str,
    select: BodyRange,
) -> PlaceholderInfo {
    PlaceholderInfo {
        evaluated: false,
        ..entry(name, summary, insert, select)
    }
}

const fn range(start: u32, end: u32) -> BodyRange {
    BodyRange { start, end }
}

const CATALOGUE: &[PlaceholderInfo] = &[
    entry(
        "date",
        "Today's date, in a format you choose",
        "{{date: %Y-%m-%d}}",
        range(8, 16),
    ),
    entry(
        "time",
        "The time now, in a format you choose",
        "{{time: %H:%M}}",
        range(8, 13),
    ),
    entry(
        "clipboard",
        "What is on the clipboard",
        "{{clipboard}}",
        range(0, 0),
    ),
    entry(
        "field",
        "A box to fill in before the snippet goes out",
        "{{field: name}}",
        range(9, 13),
    ),
    entry(
        "choice",
        "A drop-down of answers to pick from",
        "{{choice: name | options: a, b, c}}",
        range(10, 14),
    ),
    entry(
        "snippet",
        "Another snippet, inlined here",
        "{{snippet: abbreviation}}",
        range(11, 23),
    ),
    entry(
        "cursor",
        "Where the cursor is left afterwards",
        "{{cursor}}",
        range(0, 0),
    ),
    entry(
        "key",
        "A Tab or Return the app acts on, not a character",
        "{{key: tab}}",
        range(7, 10),
    ),
    pending(
        "ai",
        "Text a model writes from a prompt",
        "{{ai: Write a reply | fallback: }}",
        range(6, 19),
    ),
    pending(
        "selection",
        "The text that was selected. AI blocks only, for now",
        "{{selection}}",
        range(0, 0),
    ),
    pending(
        "app",
        "The app being typed into. AI blocks only, for now",
        "{{app}}",
        range(0, 0),
    ),
    pending(
        "window",
        "The window being typed into. AI blocks only, for now",
        "{{window}}",
        range(0, 0),
    ),
];

/// The catalogue's entry for `name`, when it has one.
pub fn known(name: &str) -> Option<&'static PlaceholderInfo> {
    catalogue().iter().find(|entry| entry.name == name)
}

/// What an editor draws over `body`.
///
/// No library, so a `{{snippet: …}}` is not checked: a reference stays as
/// written rather than being reported missing while it is still being typed.
/// The core has [`outline_with`] for an editor that can look one up.
///
/// The work is one parse and one walk of the body, so a shell can ask on every
/// keystroke.
pub fn outline(body: &str) -> BodyOutline {
    outline_with(body, None)
}

/// The same, with somewhere to look nested snippets up. A reference that is
/// not in the library, or that leads back to the body itself, is reported
/// here, at the `{{snippet: …}}` that names it.
pub fn outline_with(body: &str, snippets: Option<&dyn Snippets>) -> BodyOutline {
    let resolved = resolve(body, snippets);
    let mut wanted: Vec<usize> = Vec::with_capacity(resolved.diagnostics().len() * 2 + 8);
    for placeholder in resolved.placeholders() {
        wanted.push(placeholder.span.start);
        wanted.push(placeholder.span.end);
    }
    for diagnostic in resolved.diagnostics() {
        wanted.push(diagnostic.span.start);
        wanted.push(diagnostic.span.end);
    }
    let units = utf16_offsets(body, wanted);

    let mut problems: Vec<BodyProblem> = resolved
        .diagnostics()
        .iter()
        .map(|diagnostic| BodyProblem {
            level: diagnostic.level(),
            message: diagnostic.message(),
            range: range_of(&diagnostic.span, &units),
        })
        .collect();
    let placeholders: Vec<BodyPlaceholder> = resolved
        .placeholders()
        .map(|placeholder: &Placeholder| {
            let entry = known(&placeholder.name);
            BodyPlaceholder {
                name: placeholder.name.clone(),
                known: entry.is_some(),
                evaluated: entry.is_some_and(|entry| entry.evaluated),
                range: range_of(&placeholder.span, &units),
            }
        })
        .collect();

    // Source order, and an error before a note that starts where it does: the
    // editor draws them in this order and the last underline is the one a
    // reader sees. The sort is stable, so the parser's diagnostics keep their
    // place ahead of what reading the placeholders found.
    problems.sort_by_key(|problem| (problem.range.start, problem.range.end));
    BodyOutline {
        placeholders,
        problems,
    }
}

/// Where each byte offset in `wanted` lands once the body ahead of it is
/// counted in UTF-16 code units.
///
/// Every offset the parser reports is a character boundary. One that is not
/// maps to the start of the character it falls in, which keeps a range on
/// screen rather than panicking behind the bridge.
fn utf16_offsets(body: &str, mut wanted: Vec<usize>) -> BTreeMap<usize, u32> {
    wanted.sort_unstable();
    wanted.dedup();
    let mut offsets = BTreeMap::new();
    let mut units: u32 = 0;
    let mut next = wanted.into_iter().peekable();
    for (at, character) in body.char_indices() {
        while next.peek().is_some_and(|&offset| offset <= at) {
            let offset = next.next().expect("peeked");
            offsets.insert(offset, units);
        }
        units += u32::try_from(character.len_utf16()).unwrap_or(u32::MAX);
    }
    // The end of the body, and anything the parser reported past it.
    for offset in next {
        offsets.insert(offset, units);
    }
    offsets
}

fn range_of(span: &Range<usize>, offsets: &BTreeMap<usize, u32>) -> BodyRange {
    let at = |offset: &usize| offsets.get(offset).copied().unwrap_or_default();
    BodyRange {
        start: at(&span.start),
        end: at(&span.end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::DiagnosticKind;

    fn ranges(body: &str) -> Vec<(u32, u32)> {
        outline(body)
            .placeholders
            .iter()
            .map(|placeholder| (placeholder.range.start, placeholder.range.end))
            .collect()
    }

    #[test]
    fn a_plain_body_has_nothing_to_draw() {
        let outline = outline("Best regards,\nSam");
        assert!(outline.placeholders.is_empty());
        assert!(outline.problems.is_empty());
        assert_eq!(outline, BodyOutline::default());
    }

    #[test]
    fn placeholders_carry_their_range_and_whether_the_name_is_known() {
        let outline = outline("Hi {{field: name}}, sent {{date}} by {{nonsense}}");
        let names: Vec<_> = outline
            .placeholders
            .iter()
            .map(|p| (p.name.as_str(), p.known))
            .collect();
        assert_eq!(
            names,
            [("field", true), ("date", true), ("nonsense", false)]
        );
        assert_eq!(
            outline.placeholders[0].range,
            BodyRange { start: 3, end: 18 }
        );
        assert_eq!(
            outline.placeholders[1].range,
            BodyRange { start: 25, end: 33 }
        );
        // A known name Aralo expands is marked so; an unknown one is not.
        let evaluated: Vec<_> = outline.placeholders.iter().map(|p| p.evaluated).collect();
        assert_eq!(evaluated, [true, true, false]);
    }

    #[test]
    fn an_unclosed_opening_is_an_error_over_the_rest_of_the_body() {
        let outline = outline("before {{date: %Y");
        assert!(outline.placeholders.is_empty());
        assert_eq!(outline.problems.len(), 1);
        let problem = &outline.problems[0];
        assert_eq!(problem.level, ProblemLevel::Error);
        assert_eq!(problem.range, BodyRange { start: 7, end: 17 });
        assert_eq!(problem.message, DiagnosticKind::Unclosed.message(None));
    }

    #[test]
    fn an_invalid_name_is_an_error_over_the_whole_placeholder() {
        for body in ["{{Date}}", "{{ }}", "{{two words}}"] {
            let outline = outline(body);
            assert!(outline.placeholders.is_empty(), "{body}");
            assert_eq!(outline.problems.len(), 1, "{body}");
            assert_eq!(outline.problems[0].level, ProblemLevel::Error, "{body}");
            assert_eq!(
                outline.problems[0].range,
                BodyRange {
                    start: 0,
                    end: u32::try_from(body.chars().count()).unwrap()
                },
                "{body}"
            );
        }
    }

    #[test]
    fn an_unknown_name_is_a_note_and_the_placeholder_stays() {
        let outline = outline("{{nonsense}}");
        assert_eq!(outline.placeholders.len(), 1);
        assert!(!outline.placeholders[0].known);
        assert_eq!(outline.problems.len(), 1);
        assert_eq!(outline.problems[0].level, ProblemLevel::Note);
        assert_eq!(outline.problems[0].range, BodyRange { start: 0, end: 12 });
        assert!(outline.problems[0].message.contains("nonsense"));
    }

    #[test]
    fn a_malformed_option_is_an_error_and_the_placeholder_stays() {
        let outline = outline("{{date: %Y | oops}}");
        assert_eq!(outline.placeholders.len(), 1);
        assert_eq!(outline.problems.len(), 1);
        assert_eq!(outline.problems[0].level, ProblemLevel::Error);
        assert_eq!(
            outline.problems[0].message,
            DiagnosticKind::MalformedOption.message(None)
        );
    }

    #[test]
    fn problems_are_in_source_order_with_errors_first() {
        let outline = outline("{{nonsense}} {{date: %Y | oops}} {{broken");
        let levels: Vec<_> = outline.problems.iter().map(|p| p.level).collect();
        assert_eq!(
            levels,
            [ProblemLevel::Note, ProblemLevel::Error, ProblemLevel::Error]
        );
        let starts: Vec<_> = outline.problems.iter().map(|p| p.range.start).collect();
        assert_eq!(starts, [0, 13, 33]);
    }

    #[test]
    fn an_error_comes_before_a_note_that_starts_where_it_does() {
        // `{{Nonsense}}` is an invalid name, so it is an error and no
        // placeholder: nothing else can start at 0. A body where both land on
        // one range is what this guards, so build it the only way there is.
        let outline = outline("{{nonsense: %Y | oops}}");
        let levels: Vec<_> = outline.problems.iter().map(|p| p.level).collect();
        assert_eq!(levels, [ProblemLevel::Error, ProblemLevel::Note]);
        assert_eq!(outline.problems[0].range, outline.problems[1].range);
    }

    #[test]
    fn ranges_are_utf16_code_units_not_bytes_or_characters() {
        // "grüße 👋 " is 8 characters, 9 UTF-16 code units (the wave is a
        // surrogate pair) and 13 bytes. A text view counts the middle one.
        let body = "grüße 👋 {{cursor}} später";
        assert_eq!(ranges(body), [(9, 19)]);
        assert_eq!(body.find("{{cursor}}"), Some(13));
    }

    #[test]
    fn an_escaped_opening_is_not_a_placeholder() {
        let outline = outline("Use \\{{date}} for dates");
        assert!(outline.placeholders.is_empty());
        assert!(outline.problems.is_empty());
    }

    #[test]
    fn every_catalogue_entry_inserts_a_placeholder_of_its_own_name() {
        for entry in catalogue() {
            let outline = outline(entry.insert);
            if entry.evaluated {
                assert_eq!(outline.problems, [], "{}", entry.name);
            } else {
                // A placeholder Aralo does not expand yet says so once, as
                // advice: the body still reads the way it was written.
                assert_eq!(outline.problems.len(), 1, "{}", entry.name);
                assert_eq!(
                    outline.problems[0].level,
                    ProblemLevel::Note,
                    "{}",
                    entry.name
                );
            }
            assert_eq!(outline.placeholders.len(), 1, "{}", entry.name);
            let placeholder = &outline.placeholders[0];
            assert_eq!(placeholder.name, entry.name);
            assert!(placeholder.known, "{}", entry.name);
            assert_eq!(
                placeholder.range,
                BodyRange {
                    start: 0,
                    end: u32::try_from(entry.insert.encode_utf16().count()).unwrap()
                },
                "{}",
                entry.name
            );
            assert_eq!(known(entry.name), Some(entry));
        }
    }

    #[test]
    fn every_catalogue_selection_lands_inside_what_it_inserts() {
        for entry in catalogue() {
            let units: Vec<u16> = entry.insert.encode_utf16().collect();
            let length = u32::try_from(units.len()).unwrap();
            assert!(entry.select.start <= entry.select.end, "{}", entry.name);
            assert!(entry.select.end <= length, "{}", entry.name);
            if entry.select.is_empty() {
                continue;
            }
            let selected =
                String::from_utf16(&units[entry.select.start as usize..entry.select.end as usize])
                    .expect("a selection of valid UTF-16 is valid UTF-16");
            // What a user replaces is a word of their own, never the syntax.
            assert!(
                !selected.contains('{') && !selected.contains('}') && !selected.contains('|'),
                "{} selects {selected:?}",
                entry.name
            );
            assert!(
                selected.trim() == selected,
                "{} selects {selected:?}",
                entry.name
            );
            assert!(!selected.is_empty(), "{}", entry.name);
        }
    }

    #[test]
    fn a_body_far_longer_than_its_placeholders_still_maps_every_range() {
        let filler = "ß".repeat(500);
        let body = format!("{filler}{{{{cursor}}}}{filler}");
        assert_eq!(ranges(&body), [(500, 510)]);
    }
}
