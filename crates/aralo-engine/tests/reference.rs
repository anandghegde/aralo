//! Property test: for any abbreviation set and any key sequence the engine
//! gives the same verdicts as a naive reference matcher.
//!
//! The reference shares no code with the engine. It keeps typed text in a
//! `Vec<char>`, scans every abbreviation on every key and applies the rules of
//! `Engine::on_key` in the most literal way possible.

use std::sync::Arc;

use aralo_engine::{
    Abbreviation, CaseMode, CasePattern, Engine, ExpansionRecord, InsertMethod, KeyEvent,
    KeyVerdict, ResetReason, Scope, SnapshotBuilder, SnippetId, Trigger, CAPACITY,
};
use proptest::prelude::*;

const APPS: [&str; 3] = ["com.apple.mail", "com.apple.Terminal", "com.example.Editor"];

#[derive(Debug, Clone)]
enum Action {
    Key(KeyEvent),
    Reset,
    FrontApp(usize),
    /// Report the pending match as inserted, which arms undo.
    Done,
}

/// (length, specificity, immediate?, reversed id): the largest wins.
type Rank = (usize, u8, bool, std::cmp::Reverse<SnippetId>);

struct Reference {
    abbreviations: Vec<Abbreviation>,
    typed: Vec<char>,
    front_app: String,
    last: Option<(SnippetId, String, bool)>,
}

impl Reference {
    fn lower(c: char) -> String {
        let lower: String = c.to_lowercase().collect();
        if lower.chars().count() == 1 {
            lower
        } else {
            c.to_string()
        }
    }

    fn same_folded(a: &[char], b: &[char]) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(&x, &y)| Self::lower(x) == Self::lower(y))
    }

    fn in_scope(&self, scope: &Scope) -> bool {
        let front = self.front_app.to_ascii_lowercase();
        let listed = |apps: &[String]| apps.iter().any(|a| a.to_ascii_lowercase() == front);
        match scope {
            Scope::Everywhere => true,
            Scope::Only(apps) => listed(apps),
            Scope::Except(apps) => !listed(apps),
        }
    }

    fn specificity(scope: &Scope) -> u8 {
        match scope {
            Scope::Everywhere => 0,
            Scope::Except(_) => 1,
            Scope::Only(_) => 2,
        }
    }

    fn pattern(typed: &[char], defined: &[char]) -> CasePattern {
        if typed == defined {
            return CasePattern::AsDefined;
        }
        let letters: Vec<char> = typed
            .iter()
            .copied()
            .filter(|c| c.is_uppercase() || c.is_lowercase())
            .collect();
        match letters.as_slice() {
            [] => CasePattern::AsDefined,
            [first, ..] if !first.is_uppercase() => CasePattern::AsDefined,
            [_] => CasePattern::Title,
            all if all.iter().all(|c| c.is_uppercase()) => CasePattern::Upper,
            _ => CasePattern::Title,
        }
    }

    fn clear(&mut self) {
        self.typed.clear();
        self.last = None;
    }

    fn on_key(&mut self, event: KeyEvent) -> KeyVerdict {
        let last = self.last.take();
        let c = match event {
            KeyEvent::Char(c) => c,
            KeyEvent::Backspace | KeyEvent::Undo => {
                if let Some((_, retype, true)) = last {
                    self.typed.clear();
                    return KeyVerdict::UndoLast {
                        delete_count: 7,
                        retype,
                        method: InsertMethod::Typed,
                    };
                }
                if event == KeyEvent::Backspace {
                    self.typed.pop();
                } else {
                    self.typed.clear();
                }
                return KeyVerdict::Pass;
            }
        };

        let before = self.typed.clone();
        self.typed.push(c);
        if self.typed.len() > CAPACITY {
            self.typed.remove(0);
        }
        let after = self.typed.clone();

        let mut best: Option<(Rank, &Abbreviation)> = None;
        for abbreviation in &self.abbreviations {
            let defined: Vec<char> = abbreviation.text.chars().collect();
            let (text, is_immediate) = match abbreviation.trigger {
                Trigger::Immediate => (&after, true),
                Trigger::Delimiter => {
                    if !abbreviation.delimiters.contains(&c) {
                        continue;
                    }
                    (&before, false)
                }
            };
            if text.len() < defined.len() {
                continue;
            }
            let (head, tail) = text.split_at(text.len() - defined.len());
            let case_ok = match abbreviation.case {
                CaseMode::Exact => tail == defined.as_slice(),
                CaseMode::Ignore | CaseMode::Adaptive => Self::same_folded(tail, &defined),
            };
            let word_ok = !abbreviation.whole_word
                || head
                    .last()
                    .is_none_or(|p| !(p.is_alphanumeric() || *p == '_'));
            if !(case_ok && word_ok && self.in_scope(&abbreviation.scope)) {
                continue;
            }
            let key = (
                defined.len(),
                Self::specificity(&abbreviation.scope),
                is_immediate,
                std::cmp::Reverse(abbreviation.snippet_id),
            );
            if best.as_ref().is_none_or(|(current, _)| key > *current) {
                best = Some((key, abbreviation));
            }
        }

        let Some(((length, _, is_immediate, _), abbreviation)) = best else {
            return KeyVerdict::Pass;
        };
        let defined: Vec<char> = abbreviation.text.chars().collect();
        let source = if is_immediate { &after } else { &before };
        let typed_abbreviation = &source[source.len() - length..];
        let case = match abbreviation.case {
            CaseMode::Adaptive => Self::pattern(typed_abbreviation, &defined),
            _ => CasePattern::AsDefined,
        };
        let mut retype: String = typed_abbreviation.iter().collect();
        if !is_immediate {
            retype.push(c);
        }
        let verdict = KeyVerdict::Match {
            snippet_id: abbreviation.snippet_id,
            delete_count: (if is_immediate { length - 1 } else { length }) as u32,
            consume: true,
            case,
            trailing: (!is_immediate && abbreviation.keep_delimiter).then_some(c),
        };
        self.last = Some((abbreviation.snippet_id, retype, false));
        self.typed.clear();
        verdict
    }
}

/// A small alphabet makes accidental matches, shared prefixes and ties common.
fn key_char() -> impl Strategy<Value = char> {
    prop_oneof![
        4 => prop::sample::select(vec!['a', 'b', 'A', 'B', ';']),
        2 => prop::sample::select(vec![' ', '.', '_', '1']),
        1 => prop::sample::select(vec!['ä', 'Ä', 'İ', '😀']),
    ]
}

fn scope() -> impl Strategy<Value = Scope> {
    let apps = prop::collection::vec(prop::sample::select(APPS.to_vec()), 1..3)
        .prop_map(|apps| apps.into_iter().map(String::from).collect::<Vec<_>>());
    prop_oneof![
        3 => Just(Scope::Everywhere),
        1 => apps.clone().prop_map(Scope::Only),
        1 => apps.prop_map(Scope::Except),
    ]
}

fn abbreviation() -> impl Strategy<Value = Abbreviation> {
    (
        0u128..6, // made unique in the test body; the order stays random
        prop::collection::vec(key_char(), 1..4),
        prop::bool::ANY,
        prop::sample::select(vec![CaseMode::Exact, CaseMode::Ignore, CaseMode::Adaptive]),
        prop::bool::ANY,
        prop::bool::ANY,
        prop::collection::vec(prop::sample::select(vec![' ', '.', ';', 'b']), 1..3),
        scope(),
    )
        .prop_map(
            |(id, text, immediate, case, whole_word, keep_delimiter, delimiters, scope)| {
                Abbreviation {
                    snippet_id: SnippetId(id),
                    text: text.into_iter().collect(),
                    trigger: if immediate {
                        Trigger::Immediate
                    } else {
                        Trigger::Delimiter
                    },
                    case,
                    whole_word,
                    keep_delimiter,
                    delimiters,
                    scope,
                }
            },
        )
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        20 => key_char().prop_map(|c| Action::Key(KeyEvent::Char(c))),
        3 => Just(Action::Key(KeyEvent::Backspace)),
        1 => Just(Action::Key(KeyEvent::Undo)),
        1 => Just(Action::Reset),
        1 => (0..APPS.len()).prop_map(Action::FrontApp),
        3 => Just(Action::Done),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn engine_agrees_with_the_reference_matcher(
        abbreviations in prop::collection::vec(abbreviation(), 0..12),
        actions in prop::collection::vec(action(), 0..200),
    ) {
        // Ids are unique but in no relation to insertion order, so the
        // lowest-id tie-break is exercised without ever being ambiguous.
        let mut abbreviations = abbreviations;
        for (index, abbreviation) in abbreviations.iter_mut().enumerate() {
            abbreviation.snippet_id = SnippetId(abbreviation.snippet_id.0 * 16 + index as u128);
        }

        let mut builder = SnapshotBuilder::new();
        for abbreviation in &abbreviations {
            builder.add(abbreviation.clone());
        }
        let (snapshot, rejections) = builder.build();
        prop_assert!(rejections.is_empty());

        let mut engine = Engine::new();
        engine.set_snapshot(Arc::new(snapshot));
        engine.set_front_app(APPS[0]);
        let mut reference = Reference {
            abbreviations,
            typed: Vec::new(),
            front_app: APPS[0].to_string(),
            last: None,
        };

        let mut pending: Option<SnippetId> = None;
        for (step, action) in actions.into_iter().enumerate() {
            match action {
                Action::Key(event) => {
                    let expected = reference.on_key(event);
                    let actual = engine.on_key(event);
                    prop_assert_eq!(&actual, &expected, "step {}: {:?}", step, event);
                    pending = match actual {
                        KeyVerdict::Match { snippet_id, .. } => Some(snippet_id),
                        _ => None,
                    };
                }
                Action::Reset => {
                    engine.reset(ResetReason::Manual);
                    reference.clear();
                    pending = None;
                }
                Action::FrontApp(index) => {
                    if !reference.front_app.eq_ignore_ascii_case(APPS[index]) {
                        reference.front_app = APPS[index].to_string();
                        reference.clear();
                        pending = None;
                    }
                    engine.set_front_app(APPS[index]);
                }
                Action::Done => {
                    if let Some(snippet_id) = pending {
                        engine.expansion_done(ExpansionRecord {
                            snippet_id,
                            delete_count: 7,
                            method: InsertMethod::Typed,
                        });
                        if let Some(last) = &mut reference.last {
                            last.2 = true;
                        }
                    }
                }
            }
        }
    }
}
