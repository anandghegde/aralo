//! Evaluating a body: what each placeholder puts in, and what a shell has to
//! ask for first.
//!
//! Evaluation is two steps, because the middle of it belongs to the shell:
//!
//! 1. [`resolve`] reads the body, pulls in the snippets it nests, collects the
//!    form it asks the user to fill in, and says which context it needs. It
//!    takes no clipboard and no answers, so an editor can run it on every
//!    keystroke and report everything that is wrong with a body before
//!    anything is typed.
//! 2. [`render`] turns that into text, with a [`Context`] holding the clock,
//!    the answers, whatever context the shell went and fetched, and what the
//!    model wrote for each `{{ai}}` block.
//!
//! Nothing here reads a clock, a clipboard or a file, or asks a model: a
//! golden file pins an expansion to the minute (ADR-0014), and a model's
//! answer arrives as an argument like everything else.

use std::collections::BTreeMap;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::datetime::{self, CivilTime};
use crate::parse::{parse, Diagnostic, DiagnosticKind, Node, Placeholder};
use crate::plan::Key;

/// How many snippets deep `{{snippet: …}}` goes before Aralo stops (PRD D7).
pub const MAX_SNIPPET_DEPTH: usize = 8;

/// The tallest box a form asks for, in lines. A body may write a bigger number;
/// the box stops growing here and scrolls instead, because a panel taller than
/// the screen is a panel with no buttons on it.
pub const MAX_FIELD_LINES: u32 = 20;

/// What the user filled in, by field name.
pub type Answers = BTreeMap<String, String>;

/// What each `{{ai}}` block came to, by [`AiBlock::index`].
pub type AiAnswers = BTreeMap<usize, AiAnswer>;

/// What one `{{ai}}` block puts in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiAnswer {
    /// Text a model wrote, as the user accepted it. It goes in as literal
    /// text: it is never read for placeholders, so a model that writes
    /// `{{clipboard}}` puts in those eleven characters and reads nothing
    /// (PRD P10).
    Text(String),
    /// No model answered: AI is off, the network failed, the user chose the
    /// fallback. The block puts in its `fallback:` text, and the expansion
    /// says why.
    Fallback { reason: String },
}

static NOTHING: ContextValues = ContextValues {
    clipboard: None,
    selection: None,
    app: None,
    window: None,
};
static NO_ANSWERS: Answers = BTreeMap::new();
static NO_AI: AiAnswers = BTreeMap::new();

/// Where `{{snippet: name-or-id}}` looks. The core implements it over the open
/// library; this crate keeps no library of its own.
pub trait Snippets {
    /// The snippet `reference` names: an id, an abbreviation or a label.
    fn body(&self, reference: &str) -> Option<Nested>;
}

/// One snippet another one nests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nested {
    /// The same string for every reference to one snippet, whichever name was
    /// used: it is what a cycle is spotted by.
    pub key: String,
    pub body: String,
}

/// Something about the moment of the expansion that only the shell can fetch.
///
/// A snippet that asks for none of these expands on the keystroke path. One
/// that asks for any of them suspends until the shell has been asked, which is
/// what keeps an undeclared kind from ever being read (PRD P4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContextKind {
    Clipboard,
    Selection,
    App,
    Window,
}

impl ContextKind {
    /// The placeholder name, which is also what a diagnostic calls it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Clipboard => "clipboard",
            Self::Selection => "selection",
            Self::App => "app",
            Self::Window => "window",
        }
    }
}

/// What the shell answered for the kinds a body asked for. A kind that was
/// never asked for stays `None`, and a placeholder that wanted it stays as
/// written rather than expanding to nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextValues {
    pub clipboard: Option<String>,
    pub selection: Option<String>,
    pub app: Option<String>,
    pub window: Option<String>,
}

impl ContextValues {
    pub fn get(&self, kind: ContextKind) -> Option<&str> {
        match kind {
            ContextKind::Clipboard => self.clipboard.as_deref(),
            ContextKind::Selection => self.selection.as_deref(),
            ContextKind::App => self.app.as_deref(),
            ContextKind::Window => self.window.as_deref(),
        }
    }
}

/// Everything one expansion needs from outside this crate.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// The local wall-clock time the expansion happens at.
    pub now: CivilTime,
    /// The locale a `{{date}}` with no `locale:` option reads in. A tag Aralo
    /// does not carry falls back to English without saying so: the user did
    /// not ask for it here, the system did.
    pub locale: &'a str,
    pub values: &'a ContextValues,
    pub answers: &'a Answers,
    /// What the `{{ai}}` blocks came to. A block with nothing here has not
    /// been run, which is what an editor's preview is: it shows the fallback.
    pub ai: &'a AiAnswers,
}

impl Context<'static> {
    /// A context with a clock and nothing else, for a body that needs nothing
    /// else.
    pub fn at(now: CivilTime) -> Self {
        Self {
            now,
            locale: "",
            values: &NOTHING,
            answers: &NO_ANSWERS,
            ai: &NO_AI,
        }
    }
}

impl<'a> Context<'a> {
    #[must_use]
    pub fn in_locale(mut self, locale: &'a str) -> Self {
        self.locale = locale;
        self
    }

    #[must_use]
    pub fn with_values(mut self, values: &'a ContextValues) -> Self {
        self.values = values;
        self
    }

    #[must_use]
    pub fn with_answers(mut self, answers: &'a Answers) -> Self {
        self.answers = answers;
        self
    }

    #[must_use]
    pub fn with_ai(mut self, ai: &'a AiAnswers) -> Self {
        self.ai = ai;
        self
    }
}

/// A body with its nested snippets pulled in, and everything that can be said
/// about it before the clock is read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Resolved {
    pieces: Vec<Piece>,
    form: Form,
    needs: Vec<ContextKind>,
    cursor: CursorPlan,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Placeholder {
        placeholder: Placeholder,
        /// The `{{…}}` as it was written: what goes in when Aralo cannot read
        /// the placeholder, so a body never silently loses a part of itself.
        source: String,
    },
}

/// Which cursor stops an expansion uses, by the piece they stand before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CursorPlan {
    #[default]
    None,
    Caret(usize),
    Selection(usize, usize),
}

impl CursorPlan {
    fn stops_at(self, index: usize) -> bool {
        match self {
            Self::None => false,
            Self::Caret(at) => at == index,
            Self::Selection(from, to) => from == index || to == index,
        }
    }

    fn kind(self) -> Cursor {
        match self {
            Self::None => Cursor::None,
            Self::Caret(_) => Cursor::Caret,
            Self::Selection(..) => Cursor::Selection,
        }
    }
}

/// Where an expansion leaves the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cursor {
    /// After the inserted text, which is where typing leaves it.
    #[default]
    None,
    /// At one place inside the text.
    Caret,
    /// With one stretch of the text selected.
    Selection,
}

/// A form a snippet asks the user to fill in before it expands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Form {
    pub fields: Vec<FormField>,
}

impl Form {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn field(&self, name: &str) -> Option<&FormField> {
        self.fields.iter().find(|field| field.name == name)
    }

    /// What the form answers when nobody fills it in: every field's default.
    /// It is what a preview shows, so a preview is a real expansion.
    pub fn defaults(&self) -> Answers {
        self.fields
            .iter()
            .map(|field| (field.name.clone(), field.default().to_owned()))
            .collect()
    }
}

/// One box in the form. A name used twice in a body is one field, and the
/// first one in the body is the one that shapes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    pub name: String,
    /// What to write beside the box: the `label:` option, or the name.
    pub label: String,
    pub kind: FieldKind,
    /// The `default:` option, empty when there is none.
    pub default: String,
    /// Byte range of the placeholder that asked for it, for an editor.
    pub span: Range<usize>,
}

impl FormField {
    /// The answer to start with: the default, or the first choice when a
    /// drop-down has no default, because a drop-down always stands on one.
    pub fn default(&self) -> &str {
        match (&self.kind, self.default.is_empty()) {
            (FieldKind::Choice { options }, true) => {
                options.first().map(String::as_str).unwrap_or_default()
            }
            _ => &self.default,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    /// A box to type one line in.
    Line,
    /// A box `lines` tall, for an answer with line breaks in it: what
    /// `| lines: 4` asks for.
    Area { lines: u32 },
    /// A drop-down.
    Choice { options: Vec<String> },
}

/// One `{{ai: prompt | fallback: … | model: …}}` block, as a session runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiBlock {
    /// Which block this is, counting every `{{ai}}` in the expansion from
    /// zero in the order they appear, nested snippets' included. It is what
    /// an [`AiAnswer`] is filed under.
    pub index: usize,
    /// What the model is asked to write. The body's own words, so trusted.
    pub prompt: String,
    /// The `fallback:` option: what goes in when no model answers. `None`
    /// when the body gave none, which puts in nothing; `Some("")` is the
    /// same, said on purpose.
    pub fallback: Option<String>,
    /// The `model:` option, which wins over the snippet's and the profile's.
    pub model: Option<String>,
    /// Byte range in the outermost body, as for every other placeholder.
    pub span: Range<usize>,
}

impl Resolved {
    /// The form to put in front of the user, empty when there is none.
    pub fn form(&self) -> &Form {
        &self.form
    }

    /// The context kinds this body asks for, in the order they first appear.
    /// Nothing else is ever requested from the shell.
    pub fn needs(&self) -> &[ContextKind] {
        &self.needs
    }

    /// Problems in the body: the parser's, and everything resolving found.
    /// The body still expands; the ranges are bytes into the outermost body.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor.kind()
    }

    /// Every `{{…}}` the body holds, in the order they appear. A placeholder
    /// from a nested snippet carries the span of the `{{snippet}}` that pulled
    /// it in, so every range points at text the user can see.
    pub fn placeholders(&self) -> impl Iterator<Item = &Placeholder> {
        self.pieces.iter().filter_map(|piece| match piece {
            Piece::Placeholder { placeholder, .. } => Some(placeholder),
            Piece::Text(_) => None,
        })
    }

    /// The `{{ai}}` blocks there are a model to ask for, in the order they
    /// appear. A block with nothing to ask is left out: it is never run, and
    /// puts in its fallback (the editor says why).
    pub fn ai_blocks(&self) -> Vec<AiBlock> {
        self.placeholders()
            .filter(|placeholder| placeholder.name == "ai")
            .enumerate()
            .filter_map(|(index, placeholder)| {
                let prompt = placeholder.argument.as_deref().map(str::trim)?;
                if prompt.is_empty() {
                    return None;
                }
                Some(AiBlock {
                    index,
                    prompt: prompt.to_owned(),
                    fallback: placeholder.option("fallback").map(str::to_owned),
                    model: placeholder
                        .option("model")
                        .map(str::trim)
                        .filter(|model| !model.is_empty())
                        .map(str::to_owned),
                    span: placeholder.span.clone(),
                })
            })
            .collect()
    }

    /// True when the body is literal text: nothing to ask for, nothing to
    /// read, and the same plan every time.
    pub fn is_static(&self) -> bool {
        self.pieces
            .iter()
            .all(|piece| matches!(piece, Piece::Text(_)))
    }
}

/// Reads `body`, inlining the snippets it nests.
///
/// `snippets` is where `{{snippet: …}}` looks. Pass `None` when there is no
/// library to look in, as an editor drawing a draft has: a nested snippet then
/// stays as written and is not reported missing.
pub fn resolve(body: &str, snippets: Option<&dyn Snippets>) -> Resolved {
    let mut resolved = Resolved::default();
    let mut stack: Vec<String> = Vec::new();
    walk(body, None, snippets, &mut stack, &mut resolved);
    settle_cursor(&mut resolved);
    resolved
}

/// `outer` is the span in the outermost body that pulled this one in, so every
/// range an editor gets points at text the user can see.
fn walk(
    body: &str,
    outer: Option<&Range<usize>>,
    snippets: Option<&dyn Snippets>,
    stack: &mut Vec<String>,
    out: &mut Resolved,
) {
    let template = parse(body);
    for mut diagnostic in template.diagnostics {
        if let Some(outer) = outer {
            diagnostic.span = outer.clone();
        }
        out.diagnostics.push(diagnostic);
    }
    for node in template.nodes {
        let mut placeholder = match node {
            Node::Text(text) => {
                out.pieces.push(Piece::Text(text));
                continue;
            }
            Node::Placeholder(placeholder) => placeholder,
        };
        let source = body[placeholder.span.clone()].to_owned();
        if let Some(outer) = outer {
            placeholder.span = outer.clone();
        }
        out.diagnostics.extend(lint(&placeholder));
        if placeholder.name == "snippet" {
            inline(placeholder, source, snippets, stack, out);
            continue;
        }
        match placeholder.name.as_str() {
            "field" | "choice" => collect_field(&placeholder, &mut out.form),
            // Only the kinds a body asks for are ever requested of the shell,
            // and only the ones Aralo already expands are asked for at all.
            "clipboard" if !out.needs.contains(&ContextKind::Clipboard) => {
                out.needs.push(ContextKind::Clipboard);
            }
            _ => {}
        }
        out.pieces.push(Piece::Placeholder {
            placeholder,
            source,
        });
    }
}

fn inline(
    placeholder: Placeholder,
    source: String,
    snippets: Option<&dyn Snippets>,
    stack: &mut Vec<String>,
    out: &mut Resolved,
) {
    let span = placeholder.span.clone();
    let give_up = |out: &mut Resolved, kind: Option<(DiagnosticKind, &str)>| {
        if let Some((kind, detail)) = kind {
            out.diagnostics
                .push(Diagnostic::about(kind, span.clone(), detail));
        }
        out.pieces.push(Piece::Placeholder {
            placeholder: placeholder.clone(),
            source: source.clone(),
        });
    };

    let reference = placeholder
        .argument
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    if reference.is_empty() {
        // `lint` already said a name is missing.
        give_up(out, None);
        return;
    }
    let Some(snippets) = snippets else {
        give_up(out, None);
        return;
    };
    let Some(nested) = snippets.body(&reference) else {
        give_up(out, Some((DiagnosticKind::SnippetMissing, &reference)));
        return;
    };
    if stack.contains(&nested.key) {
        give_up(out, Some((DiagnosticKind::SnippetCycle, &reference)));
        return;
    }
    if stack.len() >= MAX_SNIPPET_DEPTH {
        give_up(out, Some((DiagnosticKind::SnippetTooDeep, &reference)));
        return;
    }
    stack.push(nested.key);
    walk(&nested.body, Some(&span), Some(snippets), stack, out);
    stack.pop();
}

fn collect_field(placeholder: &Placeholder, form: &mut Form) {
    let Some(name) = placeholder.argument.as_deref().map(str::trim) else {
        return;
    };
    if name.is_empty() || form.field(name).is_some() {
        return;
    }
    let kind = match (placeholder.name.as_str(), field_lines(placeholder)) {
        ("choice", _) => FieldKind::Choice {
            options: choices(placeholder),
        },
        (_, Some(lines)) => FieldKind::Area { lines },
        (_, None) => FieldKind::Line,
    };
    form.fields.push(FormField {
        name: name.to_owned(),
        label: placeholder
            .option("label")
            .filter(|label| !label.is_empty())
            .unwrap_or(name)
            .to_owned(),
        kind,
        default: placeholder.option("default").unwrap_or_default().to_owned(),
        span: placeholder.span.clone(),
    });
}

/// `| lines: 4`: how tall a box to type in is, when the body asks for more than
/// one line. One line, or a number Aralo cannot read, is an ordinary box, and
/// `lint` says so about the number it could not read.
fn field_lines(placeholder: &Placeholder) -> Option<u32> {
    let lines: u32 = placeholder.option("lines")?.trim().parse().ok()?;
    (lines > 1).then(|| lines.min(MAX_FIELD_LINES))
}

/// `| options: a, b, c`. A comma separates, so no single choice holds one.
fn choices(placeholder: &Placeholder) -> Vec<String> {
    placeholder
        .option("options")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|choice| !choice.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Everything wrong with one placeholder that can be seen without a clock, a
/// clipboard or an answer. It is what an editor underlines.
fn lint(placeholder: &Placeholder) -> Vec<Diagnostic> {
    let span = placeholder.span.clone();
    let name = placeholder.name.as_str();
    let about = |kind: DiagnosticKind, detail: &str| Diagnostic::about(kind, span.clone(), detail);

    // `{{if}}` blocks are format v1 (PRD D6). They parse as placeholders, so
    // say what they are rather than that Aralo has never heard of them.
    if matches!(name, "if" | "else" | "end") {
        return vec![about(DiagnosticKind::NotImplemented, name)];
    }
    let Some(entry) = crate::highlight::known(name) else {
        return vec![about(DiagnosticKind::UnknownName, name)];
    };
    if name == "ai" {
        let asks = placeholder
            .argument
            .as_deref()
            .is_some_and(|prompt| !prompt.trim().is_empty());
        return if !asks {
            vec![Diagnostic::new(DiagnosticKind::MissingPrompt, span)]
        } else if placeholder.option("fallback").is_none() {
            vec![Diagnostic::new(DiagnosticKind::AiNoFallback, span)]
        } else {
            Vec::new()
        };
    }
    if !entry.evaluated {
        return vec![about(DiagnosticKind::NotImplemented, name)];
    }

    let mut found = Vec::new();
    match name {
        "date" | "time" => {
            if let Some(tag) = placeholder.option("locale") {
                if datetime::locale(tag).is_none() {
                    found.push(about(DiagnosticKind::UnknownLocale, tag));
                }
            }
            // The pattern is checked against a locale Aralo carries; every
            // locale writes the same directives, so one check covers them all.
            if let Some(pattern) = placeholder.argument.as_deref() {
                if let Err(bad) = datetime::format_time(
                    CivilTime::new(2000, 1, 1, 0, 0, 0),
                    pattern,
                    locale_or_default(placeholder.option("locale")),
                ) {
                    found.push(about(DiagnosticKind::BadFormat, &bad.directive));
                }
            }
        }
        // A name after the colon is what an answer, or a snippet, is found by.
        "field" | "choice" | "snippet" => {
            let named = placeholder
                .argument
                .as_deref()
                .map(str::trim)
                .unwrap_or_default();
            if named.is_empty() {
                found.push(about(DiagnosticKind::MissingName, name));
            } else if name == "choice" && choices(placeholder).is_empty() {
                found.push(about(DiagnosticKind::MissingOptions, named));
            }
            if name == "field" && unreadable_lines(placeholder) {
                found.push(about(DiagnosticKind::BadOption, "lines"));
            }
        }
        // Tab and Return are the keys the plan can press; anything else stays
        // as written rather than becoming a character that is not that key.
        "key" if key_named(placeholder).is_none() => {
            found.push(match placeholder.argument.as_deref().map(str::trim) {
                Some(written) if !written.is_empty() => about(DiagnosticKind::UnknownKey, written),
                _ => Diagnostic::new(DiagnosticKind::UnknownKey, span.clone()),
            });
        }
        _ => {}
    }
    found
}

/// The key `{{key: …}}` asks for, or `None` for a body that named something
/// else. `enter` is TextExpander's name for Return, and imported bodies use it.
fn key_named(placeholder: &Placeholder) -> Option<Key> {
    match placeholder
        .argument
        .as_deref()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "tab" => Some(Key::Tab),
        "return" | "enter" => Some(Key::Return),
        _ => None,
    }
}

/// Whether a `| lines:` option is there and is not a whole number, which is
/// the one case worth telling the user about: the box is one line either way.
fn unreadable_lines(placeholder: &Placeholder) -> bool {
    placeholder
        .option("lines")
        .is_some_and(|lines| lines.trim().parse::<u32>().is_err())
}

fn locale_or_default(tag: Option<&str>) -> &'static datetime::Locale {
    tag.and_then(datetime::locale)
        .unwrap_or_else(datetime::default_locale)
}

/// Decides which cursor stops an expansion uses, and says so about the rest.
///
/// A pair of `{{cursor: select}}` marks a selection and wins over a plain
/// `{{cursor}}`; otherwise the first `{{cursor}}` is where the cursor lands.
fn settle_cursor(out: &mut Resolved) {
    let mut carets = Vec::new();
    let mut selects = Vec::new();
    for (index, piece) in out.pieces.iter().enumerate() {
        let Piece::Placeholder { placeholder, .. } = piece else {
            continue;
        };
        if placeholder.name != "cursor" {
            continue;
        }
        if placeholder.argument.as_deref().map(str::trim) == Some("select") {
            selects.push(index);
        } else {
            carets.push(index);
        }
    }

    let mut ignored: Vec<usize> = Vec::new();
    let mut lone = None;
    out.cursor = if selects.len() >= 2 {
        ignored.extend(&selects[2..]);
        ignored.extend(&carets);
        CursorPlan::Selection(selects[0], selects[1])
    } else if let Some(&first) = carets.first() {
        ignored.extend(&carets[1..]);
        ignored.extend(&selects);
        CursorPlan::Caret(first)
    } else if let Some(&only) = selects.first() {
        lone = Some(only);
        CursorPlan::Caret(only)
    } else {
        CursorPlan::None
    };

    ignored.sort_unstable();
    for index in ignored.into_iter().chain(lone) {
        let Some(Piece::Placeholder { placeholder, .. }) = out.pieces.get(index) else {
            continue;
        };
        let kind = if Some(index) == lone {
            DiagnosticKind::LoneSelection
        } else {
            DiagnosticKind::ExtraCursor
        };
        out.diagnostics
            .push(Diagnostic::new(kind, placeholder.span.clone()));
    }
}

/// The text a resolved body expands to, split where the cursor stops.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rendered {
    segments: Vec<Vec<Part>>,
    cursor: Cursor,
    diagnostics: Vec<Diagnostic>,
    ai_spans: Vec<AiSpan>,
}

/// Where one `{{ai}}` block's text sits in [`Rendered::text`]: what a panel
/// marks as the model's, with everything outside it the body's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSpan {
    /// The block's [`AiBlock::index`].
    pub block: usize,
    /// A byte range of [`Rendered::text`]. Empty for a block that put in
    /// nothing, which still has a place.
    pub range: Range<usize>,
}

/// A piece of what an expansion inserts: text, or a key the app is to act on.
///
/// Text runs are merged as they are rendered, so a segment is text, then a key,
/// then text, and never two text runs in a row. Counting characters can then
/// count each run whole, which is what it takes to count what a user sees as
/// one character: an accent that follows a letter belongs to the same run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Part {
    Text(String),
    Key(Key),
}

impl Part {
    /// What the part puts in the document, with a key written as the character
    /// it types: it is what a preview shows, and what Backspace would take back
    /// if the key had been a character.
    pub(crate) fn as_text(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Key(Key::Return) => "\n",
            Self::Key(Key::Tab) => "\t",
        }
    }

    /// How far left the cursor moves to get back over this part, in what a user
    /// sees as characters. A key press is one of them, whatever the app did
    /// with it.
    pub(crate) fn width(&self) -> u32 {
        match self {
            Self::Text(text) => u32::try_from(text.graphemes(true).count()).unwrap_or(u32::MAX),
            Self::Key(_) => 1,
        }
    }
}

/// Adds text to the end of `parts`, merging it into the run already there.
pub(crate) fn push_text(parts: &mut Vec<Part>, text: &str) {
    if text.is_empty() {
        return;
    }
    match parts.last_mut() {
        Some(Part::Text(run)) => run.push_str(text),
        _ => parts.push(Part::Text(text.to_owned())),
    }
}

impl Rendered {
    /// Everything the expansion inserts, with a key press written as the
    /// character it types.
    pub fn text(&self) -> String {
        self.segments.iter().flatten().map(Part::as_text).collect()
    }

    /// The text between the cursor stops: one more than there are stops, so
    /// one part for a body with no `{{cursor}}`, two for a caret and three for
    /// a selection.
    pub fn segments(&self) -> Vec<String> {
        self.segments
            .iter()
            .map(|parts| parts.iter().map(Part::as_text).collect())
            .collect()
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// What the expansion itself found: a value nobody supplied, a format it
    /// could not write, a block no model answered. [`Resolved::diagnostics`]
    /// holds the rest.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Where each `{{ai}}` block's text went, in the order they appear.
    pub fn ai_spans(&self) -> &[AiSpan] {
        &self.ai_spans
    }

    /// The segments, to re-case and turn into steps.
    pub(crate) fn into_segments(self) -> Vec<Vec<Part>> {
        self.segments
    }
}

/// Turns a resolved body into the text it expands to.
pub fn render(resolved: &Resolved, context: Context<'_>) -> Rendered {
    let mut out = Rendered {
        segments: vec![Vec::new()],
        cursor: resolved.cursor.kind(),
        diagnostics: Vec::new(),
        ai_spans: Vec::new(),
    };
    // Bytes of `text()` so far, for the spans an AI block is given.
    let mut written = 0;
    let mut blocks = 0;
    for (index, piece) in resolved.pieces.iter().enumerate() {
        if resolved.cursor.stops_at(index) {
            out.segments.push(Vec::new());
        }
        let parts = out
            .segments
            .last_mut()
            .expect("the segments start with one");
        // A `{{key}}` is the one placeholder that is not text: it goes in as a
        // key the app can act on, which is why it was written instead of a tab
        // or a line break.
        if let Piece::Placeholder { placeholder, .. } = piece {
            if let Some(key) = key_named(placeholder).filter(|_| placeholder.name == "key") {
                let part = Part::Key(key);
                written += part.as_text().len();
                parts.push(part);
                continue;
            }
        }
        let text = match piece {
            Piece::Text(text) => text.clone(),
            Piece::Placeholder { placeholder, .. } if placeholder.name == "ai" => {
                let block = blocks;
                blocks += 1;
                let text = ai_value(placeholder, context.ai.get(&block), &mut out.diagnostics);
                out.ai_spans.push(AiSpan {
                    block,
                    range: written..written + text.len(),
                });
                text
            }
            Piece::Placeholder {
                placeholder,
                source,
            } => value(placeholder, source, context, &mut out.diagnostics),
        };
        written += text.len();
        push_text(parts, &text);
    }
    out
}

/// What an `{{ai}}` block puts in: what the model wrote, as it was written,
/// or the fallback.
///
/// The answer is pushed as text and goes nowhere near the parser, which is the
/// whole of what keeps a model from writing a placeholder into an expansion
/// (PRD P10). A block that has not been run is a preview, and shows its
/// fallback without saying anything; one that no model answered says why.
fn ai_value(
    placeholder: &Placeholder,
    answer: Option<&AiAnswer>,
    diagnostics: &mut Vec<Diagnostic>,
) -> String {
    let fallback = placeholder.option("fallback").unwrap_or_default();
    match answer {
        Some(AiAnswer::Text(text)) => text.clone(),
        Some(AiAnswer::Fallback { reason }) => {
            let kind = if fallback.is_empty() {
                DiagnosticKind::AiNothing
            } else {
                DiagnosticKind::AiFallback
            };
            diagnostics.push(Diagnostic::about(
                kind,
                placeholder.span.clone(),
                reason.as_str(),
            ));
            fallback.to_owned()
        }
        None => fallback.to_owned(),
    }
}

/// What one placeholder puts in. A placeholder Aralo cannot expand puts in the
/// `{{…}}` it was written as, so nothing is silently dropped.
fn value(
    placeholder: &Placeholder,
    source: &str,
    context: Context<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) -> String {
    let answer = |name: &str| {
        let name = name.trim();
        context
            .answers
            .get(name)
            .map(String::as_str)
            .or_else(|| placeholder.option("default"))
            .unwrap_or_default()
            .to_owned()
    };
    match placeholder.name.as_str() {
        "date" | "time" => {
            let locale = locale_or_default(placeholder.option("locale").or(Some(context.locale)));
            let pattern = placeholder.argument.as_deref().unwrap_or({
                if placeholder.name == "date" {
                    locale.date
                } else {
                    locale.time
                }
            });
            match datetime::format_time(context.now, pattern, locale) {
                Ok(text) => text,
                Err(bad) => {
                    diagnostics.push(Diagnostic::about(
                        DiagnosticKind::BadFormat,
                        placeholder.span.clone(),
                        bad.directive,
                    ));
                    source.to_owned()
                }
            }
        }
        "clipboard" | "selection" | "app" | "window" => {
            let kind = match placeholder.name.as_str() {
                "clipboard" => ContextKind::Clipboard,
                "selection" => ContextKind::Selection,
                "app" => ContextKind::App,
                _ => ContextKind::Window,
            };
            match context.values.get(kind) {
                Some(value) => value.to_owned(),
                None => {
                    // A kind Aralo does not expand yet was already reported
                    // by `lint`, in the words that say why. Saying that
                    // nothing supplied it as well tells the user the same
                    // thing twice about one placeholder.
                    if crate::highlight::known(&placeholder.name)
                        .is_some_and(|entry| entry.evaluated)
                    {
                        diagnostics.push(Diagnostic::about(
                            DiagnosticKind::NothingSupplied,
                            placeholder.span.clone(),
                            kind.name(),
                        ));
                    }
                    source.to_owned()
                }
            }
        }
        "field" | "choice" => answer(placeholder.argument.as_deref().unwrap_or_default()),
        // The stop is where this piece sits; it inserts nothing itself.
        "cursor" => String::new(),
        // A key Aralo can press never reaches here; one it cannot stays as
        // written, and `lint` said which keys it can press.
        "key" => source.to_owned(),
        // An unknown name, a `{{snippet}}` that could not be inlined, an
        // `{{if}}`: all of them stay as written, and `lint` said why.
        _ => source.to_owned(),
    }
}
