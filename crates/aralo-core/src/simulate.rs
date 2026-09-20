use aralo_engine::{Engine, ExpansionRecord, InsertMethod, KeyEvent, KeyVerdict};
use aralo_template::{Key, Step};
use unicode_segmentation::UnicodeSegmentation;

use crate::{Core, MatchInfo};

/// A text field that exists only in memory. It runs keys through the engine
/// and applies the plans exactly as a shell would, so the whole expansion path
/// can be tested, and tried from the command line, without a window server.
pub struct Simulator<'a> {
    core: &'a Core,
    engine: Engine,
    field: String,
    expansions: usize,
}

// Like the engine, never prints what was typed.
impl std::fmt::Debug for Simulator<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Simulator")
            .field("field_len", &self.field.len())
            .field("expansions", &self.expansions)
            .finish_non_exhaustive()
    }
}

impl<'a> Simulator<'a> {
    pub fn new(core: &'a Core, front_app: &str) -> Self {
        let mut engine = core.engine();
        engine.set_front_app(front_app);
        Self {
            core,
            engine,
            field: String::new(),
            expansions: 0,
        }
    }

    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// What the text field holds now.
    pub fn text(&self) -> &str {
        &self.field
    }

    pub fn expansions(&self) -> usize {
        self.expansions
    }

    pub fn type_str(&mut self, text: &str) -> &mut Self {
        for c in text.chars() {
            self.key(KeyEvent::Char(c));
        }
        self
    }

    pub fn key(&mut self, event: KeyEvent) -> &mut Self {
        match self.engine.on_key(event) {
            KeyVerdict::Pass => match event {
                KeyEvent::Char(c) => self.field.push(c),
                KeyEvent::Backspace => self.backspace(1),
                // Without an expansion to undo, the app's own undo runs; the
                // simulator has no history to model it with.
                KeyEvent::Undo => {}
            },
            KeyVerdict::Match {
                snippet_id,
                delete_count,
                case,
                trailing,
                consume,
            } => {
                let expansion = self.core.expand(MatchInfo {
                    snippet_id,
                    delete_count,
                    case,
                    trailing,
                });
                match expansion {
                    Some(expansion) => {
                        for step in &expansion.plan.steps {
                            self.apply(step);
                        }
                        self.expansions += 1;
                        if let Some(delete_count) = expansion.undo_delete_count {
                            self.engine.expansion_done(ExpansionRecord {
                                snippet_id,
                                delete_count,
                                method: InsertMethod::Typed,
                            });
                        }
                    }
                    None => {
                        if let (false, KeyEvent::Char(c)) = (consume, event) {
                            self.field.push(c);
                        }
                    }
                }
            }
            KeyVerdict::UndoLast {
                delete_count,
                retype,
                ..
            } => {
                self.backspace(delete_count);
                self.field.push_str(&retype);
            }
        }
        self
    }

    fn apply(&mut self, step: &Step) {
        match step {
            Step::Delete { count } => self.backspace(*count),
            Step::InsertText { text } => self.field.push_str(text),
            Step::InsertRich { plain, .. } => self.field.push_str(plain),
            Step::KeyPress { key: Key::Return } => self.field.push('\n'),
            Step::KeyPress { key: Key::Tab } => self.field.push('\t'),
            Step::Delay { .. } | Step::MoveCursor { .. } => {}
        }
    }

    /// Removes user-perceived characters, the unit a text field deletes by.
    fn backspace(&mut self, count: u32) {
        for _ in 0..count {
            match self.field.grapheme_indices(true).next_back() {
                Some((start, _)) => self.field.truncate(start),
                None => return,
            }
        }
    }
}
