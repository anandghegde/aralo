//! A text field that exists only in memory, with a caret and a selection.
//!
//! It runs a plan's steps the way an app's text field takes them from a shell,
//! so the [`Simulator`](crate::Simulator) and a [`Trial`](crate::Trial) show
//! what an expansion does to a document without a window server.

use std::ops::Range;

use aralo_template::{Key, Step};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Default)]
pub(crate) struct Field {
    text: String,
    /// Where the caret is, as a byte offset into `text`. Text goes in here,
    /// Backspace deletes from here, and a `{{cursor}}` stop moves it, so what
    /// the user would type next lands where they would expect.
    caret: usize,
    /// What the last expansion left selected, when it asked for a selection.
    selection: Option<Range<usize>>,
}

// Never prints what was typed.
impl std::fmt::Debug for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Field")
            .field("len", &self.text.len())
            .field("caret", &self.caret)
            .finish_non_exhaustive()
    }
}

impl Field {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn caret(&self) -> usize {
        self.caret
    }

    /// The selected byte range, `None` when nothing is selected.
    pub(crate) fn selection(&self) -> Option<Range<usize>> {
        self.selection.clone()
    }

    pub(crate) fn selected(&self) -> &str {
        match &self.selection {
            Some(range) => &self.text[range.clone()],
            None => "",
        }
    }

    /// The field with the caret written in as `|`, and a selection in
    /// brackets.
    pub(crate) fn marked(&self) -> String {
        let mut out = String::with_capacity(self.text.len() + 3);
        match &self.selection {
            Some(range) => {
                out.push_str(&self.text[..range.start]);
                out.push('[');
                out.push_str(&self.text[range.clone()]);
                out.push(']');
                out.push_str(&self.text[range.end..]);
            }
            None => {
                out.push_str(&self.text[..self.caret]);
                out.push('|');
                out.push_str(&self.text[self.caret..]);
            }
        }
        out
    }

    /// Puts the caret at `at`, a byte offset, moved back to the nearest
    /// character boundary and never past the end.
    pub(crate) fn set_caret(&mut self, at: usize) {
        let mut at = at.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at -= 1;
        }
        self.caret = at;
        self.selection = None;
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn apply(&mut self, step: &Step) {
        match step {
            Step::Delete { count } => self.backspace(*count),
            Step::InsertText { text } => self.insert_text(text),
            Step::InsertRich { plain, .. } => self.insert_text(plain),
            Step::KeyPress { key: Key::Return } => self.write('\n'),
            Step::KeyPress { key: Key::Tab } => self.write('\t'),
            Step::Delay { .. } => {}
            Step::MoveCursor { graphemes, select } => {
                let anchor = self.caret;
                self.caret = self.left_of(self.caret, *graphemes);
                // Moving left while selecting runs from where the caret was
                // back to where it is now, which is the text the plan meant to
                // put in front of the user.
                self.selection = select.then(|| self.caret..anchor);
            }
        }
    }

    /// A key typed over whatever is selected, which it replaces, as in any
    /// text field.
    pub(crate) fn write(&mut self, c: char) {
        self.take_selection();
        self.text.insert(self.caret, c);
        self.caret += c.len_utf8();
    }

    pub(crate) fn insert_text(&mut self, text: &str) {
        self.text.insert_str(self.caret, text);
        self.caret += text.len();
        self.selection = None;
    }

    /// Removes user-perceived characters before the caret, the unit a text
    /// field deletes by. A selection goes first, and counts as one.
    pub(crate) fn backspace(&mut self, count: u32) {
        let mut count = count;
        if count > 0 && self.take_selection() {
            count -= 1;
        }
        for _ in 0..count {
            let Some((start, _)) = self.text[..self.caret].grapheme_indices(true).next_back()
            else {
                break;
            };
            self.text.replace_range(start..self.caret, "");
            self.caret = start;
        }
    }

    /// Deletes the selection, if there is one, and leaves the caret where it
    /// began. True when something was deleted.
    fn take_selection(&mut self) -> bool {
        match self.selection.take() {
            Some(range) if !range.is_empty() => {
                self.text.replace_range(range.clone(), "");
                self.caret = range.start;
                true
            }
            _ => false,
        }
    }

    /// The byte offset `graphemes` user-perceived characters to the left of
    /// `from`, stopping at the start of the field.
    fn left_of(&self, from: usize, graphemes: u32) -> usize {
        let mut at = from;
        for _ in 0..graphemes {
            match self.text[..at].grapheme_indices(true).next_back() {
                Some((start, _)) => at = start,
                None => break,
            }
        }
        at
    }
}
