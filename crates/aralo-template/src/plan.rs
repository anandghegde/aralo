use unicode_segmentation::UnicodeSegmentation;

/// What the injector does for one expansion, in order. The plan says what
/// reaches the document; the shell picks how (typing or paste) per app.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExpansionPlan {
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Press Backspace `count` times.
    Delete { count: u32 },
    /// Insert plain text. May contain newlines.
    InsertText { text: String },
    /// Insert rich text, with the plain form for apps that refuse it (v1).
    InsertRich { html: String, plain: String },
    /// Press a key the target app should see as a key, not as text.
    KeyPress { key: Key },
    /// Wait; used by per-app compatibility rules.
    Delay { millis: u32 },
    /// Move the cursor left by `graphemes`, selecting on the way if `select`.
    MoveCursor { graphemes: u32, select: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Return,
    Tab,
}

impl ExpansionPlan {
    /// Backspaces that remove what this plan inserts: one per user-perceived
    /// character, the unit text fields delete by. Counting code points instead
    /// would over-delete after an emoji and eat the user's own text.
    ///
    /// `None` when the plan inserts something Backspace cannot be trusted to
    /// remove (rich text, or a Return or Tab the app may have acted on).
    pub fn undo_delete_count(&self) -> Option<u32> {
        let mut count = 0u32;
        for step in &self.steps {
            match step {
                Step::InsertText { text } => count += text.graphemes(true).count() as u32,
                Step::InsertRich { .. } | Step::KeyPress { .. } => return None,
                Step::Delete { .. } | Step::Delay { .. } | Step::MoveCursor { .. } => {}
            }
        }
        Some(count)
    }

    /// All plain text the plan inserts, for previews and tests.
    pub fn inserted_text(&self) -> String {
        let mut out = String::new();
        for step in &self.steps {
            match step {
                Step::InsertText { text } => out.push_str(text),
                Step::InsertRich { plain, .. } => out.push_str(plain),
                Step::KeyPress { key: Key::Return } => out.push('\n'),
                Step::KeyPress { key: Key::Tab } => out.push('\t'),
                Step::Delete { .. } | Step::Delay { .. } | Step::MoveCursor { .. } => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_counts_user_perceived_characters() {
        let plan = ExpansionPlan {
            steps: vec![
                Step::Delete { count: 3 },
                Step::InsertText {
                    // family emoji (one grapheme, 7 code points), e + combining acute, CRLF
                    text: "a👨‍👩‍👧‍👦e\u{301}\r\n".into(),
                },
            ],
        };
        assert_eq!(plan.undo_delete_count(), Some(4));
    }

    #[test]
    fn undo_by_backspace_is_not_offered_after_a_key_press() {
        let plan = ExpansionPlan {
            steps: vec![
                Step::InsertText { text: "hi".into() },
                Step::KeyPress { key: Key::Return },
            ],
        };
        assert_eq!(plan.undo_delete_count(), None);
        assert_eq!(plan.inserted_text(), "hi\n");
    }
}
