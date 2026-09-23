//! TextExpander macros to Aralo placeholders.
//!
//! The mapping is in `docs/format/import.md`. Two rules hold everywhere:
//!
//! * Nothing is invented. A macro with no Aralo equivalent stays in the body
//!   as the literal text it was, and the snippet gets an
//!   [`NoteKind::Unconvertible`] note, so the import report can list it and a
//!   search of the library finds every one.
//! * Nothing is silently reinterpreted. A `{{` already in the source is
//!   escaped to `\{{`, because in Aralo it would open a placeholder.

use crate::report::{Note, NoteKind};

/// Date specifiers that make a run a date rather than a time.
const DATE_SPECIFIERS: &str = "YymBbdeajA";
/// Specifiers that on their own mean a time of day.
const TIME_SPECIFIERS: &str = "HIMSpZz";
/// Characters allowed between two date specifiers in one run.
const SEPARATORS: [char; 7] = [' ', '-', '/', '.', ':', ',', '\''];
/// The longest separator run folded into a date format. Enough for `", "`,
/// short enough that a sentence is never swallowed.
const MAX_SEPARATOR: usize = 4;
/// Keys a fill-in macro's head may carry. Splitting on these rather than on
/// every colon keeps a default value that contains a colon intact.
const FILL_KEYS: [&str; 6] = ["name=", "default=", "width=", "height=", "lines=", "id="];

const OPTIONAL_SECTION: &str = "%fillpart…% (an optional section)";
/// How tall a box a fill-in area becomes when the source does not say. Enough
/// for a paragraph of case notes, which is what an area is usually for.
const AREA_LINES: u32 = 4;
const CLEANED_FOR_GRAMMAR: &str = "a value was cleaned for the placeholder grammar";

/// Converts one snippet body. The notes are deduplicated: a body with four
/// `%delay%` macros reports the macro once, so counting notes counts snippets.
pub fn convert(source: &str) -> (String, Vec<Note>) {
    let mut out = String::with_capacity(source.len() + 16);
    let mut notes = Notes::default();
    let mut rest = source;
    while let Some(index) = rest.find(['%', '{']) {
        out.push_str(&rest[..index]);
        rest = &rest[index..];
        let consumed = step(rest, &mut out, &mut notes);
        rest = &rest[consumed..];
    }
    out.push_str(rest);
    (out, notes.0)
}

/// Makes a body that is meant to be literal text stay literal: the only
/// sequence Aralo reads specially is `{{`.
pub fn escape(source: &str) -> String {
    source.replace("{{", "\\{{")
}

/// Whether a body carries TextExpander macros. This decides whether a CSV
/// column needs converting, so it asks for an unmistakable macro and never
/// for a lone percent sign: "50% off" is left alone.
pub fn looks_like_textexpander(text: &str) -> bool {
    const HEADS: [&str; 7] = [
        "%|",
        "%clipboard",
        "%key:",
        "%snippet:",
        "%fill",
        "%delay:",
        "%@",
    ];
    if HEADS.iter().any(|head| text.contains(head)) {
        return true;
    }
    // A run of two or more date specifiers, such as `%m/%d/%Y`.
    let mut rest = text;
    while let Some(index) = rest.find('%') {
        rest = &rest[index..];
        match date_run(rest) {
            Some((format, _, _)) if format.matches('%').count() >= 2 => return true,
            Some((_, length, _)) => rest = &rest[length..],
            None => rest = &rest[1..],
        }
    }
    false
}

/// Notes for one snippet, kept unique.
#[derive(Debug, Default)]
struct Notes(Vec<Note>);

impl Notes {
    fn push(&mut self, kind: NoteKind, detail: impl Into<String>) {
        let note = Note::new(kind, detail);
        if !self.0.contains(&note) {
            self.0.push(note);
        }
    }
}

/// Handles one `%` or `{` and returns how many bytes of `rest` it used.
fn step(rest: &str, out: &mut String, notes: &mut Notes) -> usize {
    if rest.starts_with("{{") {
        out.push_str("\\{{");
        return 2;
    }
    if !rest.starts_with('%') {
        out.push('{');
        return 1;
    }
    if let Some(consumed) = simple(rest, out) {
        return consumed;
    }
    if let Some(consumed) = key(rest, out, notes) {
        return consumed;
    }
    if let Some((argument, consumed)) = delimited(rest, "%snippet:") {
        let (name, changed) = placeholder_safe(argument);
        if changed {
            notes.push(NoteKind::Approximated, CLEANED_FOR_GRAMMAR);
        }
        out.push_str("{{snippet: ");
        out.push_str(&name);
        out.push_str("}}");
        return consumed;
    }
    if let Some(consumed) = fill(rest, out, notes) {
        return consumed;
    }
    if let Some((_, consumed)) = delimited(rest, "%delay:") {
        notes.push(NoteKind::Unconvertible, "%delay:…% (a typing delay)");
        out.push_str(&rest[..consumed]);
        return consumed;
    }
    if let Some(consumed) = adjusted_date(rest) {
        notes.push(NoteKind::Unconvertible, "%@ (date arithmetic)");
        out.push_str(&rest[..consumed]);
        return consumed;
    }
    if let Some((format, consumed, time_only)) = date_run(rest) {
        out.push_str(if time_only { "{{time: " } else { "{{date: " });
        out.push_str(&format);
        out.push_str("}}");
        return consumed;
    }
    // A percent sign in ordinary text.
    out.push('%');
    1
}

/// The macros that are one fixed string.
fn simple(rest: &str, out: &mut String) -> Option<usize> {
    const SIMPLE: [(&str, &str); 3] = [
        ("%%", "%"),
        ("%|", "{{cursor}}"),
        ("%clipboard", "{{clipboard}}"),
    ];
    SIMPLE.iter().find_map(|(macro_text, replacement)| {
        rest.starts_with(macro_text).then(|| {
            out.push_str(replacement);
            macro_text.len()
        })
    })
}

/// `%key:tab%`. Tab and Return are keys Aralo presses too; the rest stay
/// literal, because a key that is not a character cannot be typed as one.
fn key(rest: &str, out: &mut String, notes: &mut Notes) -> Option<usize> {
    let (name, consumed) = delimited(rest, "%key:")?;
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        // A space key types a space; nothing is lost.
        "space" => out.push(' '),
        "tab" => out.push_str("{{key: tab}}"),
        "return" | "enter" => out.push_str("{{key: return}}"),
        _ => {
            notes.push(
                NoteKind::Unconvertible,
                format!("%key:{name}% (a key press)"),
            );
            out.push_str(&rest[..consumed]);
        }
    }
    Some(consumed)
}

/// The fill-in family: text, area, popup, date, optional section, and the
/// TextExpander 3 spelling `%fill:name%`.
fn fill(rest: &str, out: &mut String, notes: &mut Notes) -> Option<usize> {
    const PART_END: &str = "%fillpartend%";
    if rest.starts_with(PART_END) {
        notes.push(NoteKind::Unconvertible, OPTIONAL_SECTION);
        out.push_str(PART_END);
        return Some(PART_END.len());
    }
    if let Some((_, consumed)) = delimited(rest, "%fillpart:") {
        notes.push(NoteKind::Unconvertible, OPTIONAL_SECTION);
        out.push_str(&rest[..consumed]);
        return Some(consumed);
    }
    if let Some((spec, consumed)) = delimited(rest, "%fillpopup:") {
        let (tail, extra) = popup_options(&rest[consumed..]);
        let head = fill_fields(spec);
        let mut values: Vec<&str> = head.default.into_iter().collect();
        values.extend(tail);
        let (name, mut changed) = placeholder_safe(head.name.unwrap_or("choice"));
        let options: Vec<String> = values
            .iter()
            .map(|value| {
                // A comma separates options, so one inside a value cannot
                // survive; it becomes a semicolon and the entry is flagged.
                let (value, cleaned) = placeholder_safe(value);
                changed |= cleaned || value.contains(',');
                value.replace(',', ";")
            })
            .collect();
        if changed {
            notes.push(NoteKind::Approximated, CLEANED_FOR_GRAMMAR);
        }
        out.push_str("{{choice: ");
        out.push_str(&name);
        out.push_str(" | options: ");
        out.push_str(&options.join(", "));
        out.push_str("}}");
        return Some(consumed + extra);
    }
    for head in ["%filltext:", "%fillarea:", "%filldate:"] {
        let Some((spec, consumed)) = delimited(rest, head) else {
            continue;
        };
        let fields = fill_fields(spec);
        let (name, mut changed) = placeholder_safe(fields.name.unwrap_or("field"));
        out.push_str("{{field: ");
        out.push_str(&name);
        if let Some(default) = fields.default {
            let (default, cleaned) = placeholder_safe(default);
            changed |= cleaned;
            out.push_str(" | default: ");
            out.push_str(&default);
        }
        // A fill-in area is a box with room to write in, which Aralo asks for
        // by the line. The source counts them when it says so, and a box whose
        // height was in pixels gets the same box a new one would.
        if head == "%fillarea:" {
            out.push_str(" | lines: ");
            out.push_str(&fields.lines.unwrap_or(AREA_LINES).to_string());
        }
        out.push_str("}}");
        if changed {
            notes.push(NoteKind::Approximated, CLEANED_FOR_GRAMMAR);
        }
        if head == "%filldate:" {
            notes.push(NoteKind::Approximated, "a date picker became a plain field");
        }
        return Some(consumed);
    }
    let (name, consumed) = delimited(rest, "%fill:")?;
    let (name, changed) = placeholder_safe(name);
    if changed {
        notes.push(NoteKind::Approximated, CLEANED_FOR_GRAMMAR);
    }
    out.push_str("{{field: ");
    out.push_str(&name);
    out.push_str("}}");
    Some(consumed)
}

/// `%name:body%`: the text between the head and the next `%`, and the whole
/// length including both delimiters.
fn delimited<'a>(rest: &'a str, head: &str) -> Option<(&'a str, usize)> {
    let after = rest.strip_prefix(head)?;
    let end = after.find('%')?;
    Some((&after[..end], head.len() + end + 1))
}

/// A popup's remaining options follow its head, each ended by `%`. They are
/// all on one line, so a newline ends the list. A literal percent sign later
/// on the same line would be read as an option; `docs/format/import.md` says
/// so, because the source format gives no way to tell them apart.
fn popup_options(mut rest: &str) -> (Vec<&str>, usize) {
    let mut options = Vec::new();
    let mut consumed = 0;
    while let Some(end) = rest.find('%') {
        if rest[..end].contains('\n') {
            break;
        }
        options.push(&rest[..end]);
        consumed += end + 1;
        rest = &rest[end + 1..];
    }
    (options, consumed)
}

#[derive(Debug, Default)]
struct FillFields<'a> {
    name: Option<&'a str>,
    default: Option<&'a str>,
    /// `lines=6` on a fill-in area, when the source counted them.
    lines: Option<u32>,
}

/// Splits `name=Customer:default=Hi there` without cutting a default that
/// contains a colon: only a colon that starts a known key is a separator.
fn fill_fields(spec: &str) -> FillFields<'_> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (index, byte) in spec.bytes().enumerate() {
        if byte == b':'
            && FILL_KEYS
                .iter()
                .any(|key| spec[index + 1..].starts_with(key))
        {
            parts.push(&spec[start..index]);
            start = index + 1;
        }
    }
    parts.push(&spec[start..]);

    let mut fields = FillFields::default();
    for part in parts {
        match part.split_once('=') {
            Some(("name", value)) => fields.name = Some(value),
            Some(("default", value)) => fields.default = Some(value),
            Some(("lines", value)) => {
                fields.lines = value.trim().parse().ok().filter(|lines| *lines > 1)
            }
            _ => {}
        }
    }
    fields
}

/// `%@+1D` and the date run after it: TextExpander's date arithmetic, which
/// Aralo v0 has no placeholder for. Returns the length of the whole thing so
/// it can be left in the body verbatim rather than converted to today's date,
/// which would be quietly wrong.
fn adjusted_date(rest: &str) -> Option<usize> {
    rest.strip_prefix("%@")?;
    let mut at = "%@".len();
    let mut moved = false;
    loop {
        let tail = &rest[at..];
        if !tail.starts_with(['+', '-']) {
            break;
        }
        let digits = tail[1..].chars().take_while(char::is_ascii_digit).count();
        let Some(unit) = tail[1 + digits..].chars().next() else {
            break;
        };
        if digits == 0 || !unit.is_ascii_alphabetic() {
            break;
        }
        at += 1 + digits + unit.len_utf8();
        moved = true;
    }
    if !moved {
        return None;
    }
    if let Some((_, length, _)) = date_run(&rest[at..]) {
        at += length;
    }
    Some(at)
}

/// One run of date and time specifiers with the separators between them, so
/// `%m/%d/%Y` becomes a single `{{date: %m/%d/%Y}}` and not three
/// placeholders with slashes in between.
///
/// Returns the format, the bytes consumed, and whether the run is a time.
fn date_run(rest: &str) -> Option<(String, usize, bool)> {
    let mut format = String::new();
    let mut at = 0;
    let mut has_date = false;
    while let Some(specifier) = specifier_at(&rest[at..]) {
        has_date |= DATE_SPECIFIERS.contains(specifier);
        format.push('%');
        format.push(specifier);
        at += '%'.len_utf8() + specifier.len_utf8();
        let separator = separator_run(&rest[at..]);
        if separator > 0 && specifier_at(&rest[at + separator..]).is_some() {
            format.push_str(&rest[at..at + separator]);
            at += separator;
        }
    }
    (!format.is_empty()).then_some((format, at, !has_date))
}

fn specifier_at(rest: &str) -> Option<char> {
    let specifier = rest.strip_prefix('%')?.chars().next()?;
    (DATE_SPECIFIERS.contains(specifier) || TIME_SPECIFIERS.contains(specifier))
        .then_some(specifier)
}

fn separator_run(rest: &str) -> usize {
    rest.chars()
        .take(MAX_SEPARATOR)
        .take_while(|character| SEPARATORS.contains(character))
        .map(char::len_utf8)
        .sum()
}

/// A value goes into a placeholder as plain text, so the characters the
/// grammar uses have to go. Returns the cleaned value and whether it changed.
fn placeholder_safe(value: &str) -> (String, bool) {
    let cleaned = value
        .replace("}}", "}")
        .replace("{{", "{")
        .replace('|', "/");
    let changed = cleaned != value;
    (cleaned, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(source: &str) -> String {
        convert(source).0
    }

    fn kinds(source: &str) -> Vec<NoteKind> {
        convert(source)
            .1
            .into_iter()
            .map(|note| note.kind)
            .collect()
    }

    #[test]
    fn plain_text_is_untouched() {
        let source = "Best regards,\nSam";
        assert_eq!(body(source), source);
        assert!(kinds(source).is_empty());
    }

    #[test]
    fn the_cursor_and_the_clipboard_convert() {
        assert_eq!(
            body("Dear %|,\n%clipboard"),
            "Dear {{cursor}},\n{{clipboard}}"
        );
        assert!(kinds("%|%clipboard").is_empty());
    }

    #[test]
    fn a_double_percent_is_one_percent() {
        assert_eq!(body("100%% sure"), "100% sure");
    }

    #[test]
    fn a_lone_percent_in_prose_stays_and_says_nothing() {
        assert_eq!(body("up 50% since May"), "up 50% since May");
        assert!(kinds("up 50% since May").is_empty());
    }

    #[test]
    fn a_date_run_folds_its_separators_into_one_placeholder() {
        assert_eq!(body("%m/%d/%Y"), "{{date: %m/%d/%Y}}");
        assert_eq!(body("%A, %B %e"), "{{date: %A, %B %e}}");
        assert_eq!(body("on %d.%m.%Y."), "on {{date: %d.%m.%Y}}.");
    }

    #[test]
    fn a_time_only_run_becomes_a_time() {
        assert_eq!(body("%H:%M"), "{{time: %H:%M}}");
        assert_eq!(body("%Y at %H:%M"), "{{date: %Y}} at {{time: %H:%M}}");
    }

    #[test]
    fn a_run_with_a_date_in_it_is_a_date() {
        assert_eq!(body("%Y-%m-%d %H:%M"), "{{date: %Y-%m-%d %H:%M}}");
    }

    #[test]
    fn date_arithmetic_is_left_alone_rather_than_made_wrong() {
        let source = "due %@+3D%m/%d/%Y";
        assert_eq!(body(source), source);
        assert_eq!(kinds(source), [NoteKind::Unconvertible]);
    }

    #[test]
    fn fill_ins_become_fields() {
        assert_eq!(body("%filltext:name=Customer%"), "{{field: Customer}}");
        assert_eq!(
            body("%filltext:name=Greeting:default=Hi there%"),
            "{{field: Greeting | default: Hi there}}"
        );
        assert_eq!(body("%fill:First name%"), "{{field: First name}}");
        assert!(kinds("%filltext:name=Customer%").is_empty());
    }

    #[test]
    fn a_default_may_contain_a_colon() {
        assert_eq!(
            body("%filltext:name=Note:default=see: below%"),
            "{{field: Note | default: see: below}}"
        );
    }

    #[test]
    fn an_area_becomes_a_box_with_room_to_write_in() {
        assert_eq!(body("%fillarea:name=Body%"), "{{field: Body | lines: 4}}");
        assert!(kinds("%fillarea:name=Body%").is_empty());
        // The source counts the lines when it says so, and says it in pixels
        // when it does not.
        assert_eq!(
            body("%fillarea:name=Body:default=Hi:lines=8%"),
            "{{field: Body | default: Hi | lines: 8}}"
        );
        assert_eq!(
            body("%fillarea:name=Body:height=120%"),
            "{{field: Body | lines: 4}}"
        );
    }

    #[test]
    fn a_date_picker_converts_but_is_flagged() {
        assert_eq!(body("%filldate:name=When%"), "{{field: When}}");
        assert_eq!(kinds("%filldate:name=When%"), [NoteKind::Approximated]);
    }

    #[test]
    fn a_popup_becomes_a_choice_with_its_default_first() {
        assert_eq!(
            body("%fillpopup:name=Size:default=Small%Medium%Large%"),
            "{{choice: Size | options: Small, Medium, Large}}"
        );
        assert!(kinds("%fillpopup:name=Size:default=S%M%L%").is_empty());
    }

    #[test]
    fn a_popup_option_list_ends_at_the_line() {
        assert_eq!(
            body("%fillpopup:name=X:default=a%b%\nnext line"),
            "{{choice: X | options: a, b}}\nnext line"
        );
    }

    #[test]
    fn a_nested_snippet_keeps_its_abbreviation() {
        assert_eq!(body("%snippet:;sig%"), "{{snippet: ;sig}}");
    }

    #[test]
    fn the_keys_aralo_presses_become_the_key_placeholder() {
        assert_eq!(body("a%key:tab%b"), "a{{key: tab}}b");
        assert_eq!(body("%key:return%"), "{{key: return}}");
        assert_eq!(body("%key:enter%"), "{{key: return}}");
        assert!(kinds("a%key:tab%b%key:enter%").is_empty());
        // A space key types a space, which is a character and not a key.
        assert_eq!(body("a%key:space%b"), "a b");
        assert!(kinds("a%key:space%b").is_empty());
    }

    #[test]
    fn any_other_key_press_stays_in_the_body_and_is_reported() {
        let source = "%key:left%%key:left%";
        assert_eq!(body(source), source);
        assert_eq!(kinds(source), [NoteKind::Unconvertible]);
    }

    #[test]
    fn a_delay_and_an_optional_section_are_reported_once_each() {
        let source = "%delay:500%%delay:200%%fillpart:name=X%maybe%fillpartend%";
        assert_eq!(body(source), source);
        assert_eq!(
            kinds(source),
            [NoteKind::Unconvertible, NoteKind::Unconvertible]
        );
    }

    #[test]
    fn braces_in_the_source_are_escaped_so_they_stay_literal() {
        assert_eq!(body("use {{name}} here"), "use \\{{name}} here");
        assert_eq!(body("a { b"), "a { b");
    }

    #[test]
    fn a_value_that_would_break_the_grammar_is_cleaned_and_flagged() {
        assert_eq!(body("%filltext:name=A|B%"), "{{field: A/B}}");
        assert_eq!(kinds("%filltext:name=A|B%"), [NoteKind::Approximated]);
        assert_eq!(
            body("%fillpopup:name=X:default=a,b%c%"),
            "{{choice: X | options: a;b, c}}"
        );
    }

    #[test]
    fn detection_needs_a_real_macro() {
        assert!(looks_like_textexpander("Dear %|"));
        assert!(looks_like_textexpander("%m/%d/%Y"));
        assert!(!looks_like_textexpander("up 50% since May"));
        assert!(!looks_like_textexpander("Best regards,"));
        // One specifier alone is not enough to call a CSV a TextExpander one.
        assert!(!looks_like_textexpander("printf %d"));
    }
}
