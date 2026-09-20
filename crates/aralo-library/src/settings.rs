use aralo_engine::{CaseMode, Scope, Trigger, DEFAULT_DELIMITERS};
use aralo_snippet::{FrontMatter, GroupFile};

/// How a snippet behaves once every level of inheritance is applied:
/// snippet, then its group's `defaults`, then each parent group, then these
/// built-in values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub trigger: Trigger,
    pub case: CaseMode,
    pub whole_word: bool,
    pub keep_delimiter: bool,
    pub delimiters: Vec<char>,
    pub scope: Scope,
    /// False when the snippet or any group above it is switched off.
    pub enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            trigger: Trigger::Delimiter,
            case: CaseMode::Adaptive,
            whole_word: true,
            keep_delimiter: true,
            delimiters: DEFAULT_DELIMITERS.to_vec(),
            scope: Scope::Everywhere,
            enabled: true,
        }
    }
}

impl Settings {
    /// The settings inside a group whose parent resolves to `self`.
    pub(crate) fn for_group(&self, group: &GroupFile) -> Settings {
        let defaults = &group.defaults;
        Settings {
            trigger: defaults.trigger.map_or(self.trigger, trigger),
            case: defaults.case.map_or(self.case, case),
            whole_word: defaults.word.unwrap_or(self.whole_word),
            keep_delimiter: defaults.keep_delimiter.unwrap_or(self.keep_delimiter),
            delimiters: match &defaults.delimiters {
                Some(delimiters) => delimiters.chars().collect(),
                None => self.delimiters.clone(),
            },
            scope: match &group.scope {
                None => self.scope.clone(),
                Some(spec) if !spec.only.is_empty() => Scope::Only(spec.only.clone()),
                Some(spec) if !spec.except.is_empty() => Scope::Except(spec.except.clone()),
                Some(_) => Scope::Everywhere,
            },
            // Off is sticky: a sub-group cannot switch itself back on.
            enabled: self.enabled && group.enabled.unwrap_or(true),
        }
    }

    /// The settings of a snippet in a group that resolves to `self`.
    pub(crate) fn for_snippet(&self, front: &FrontMatter) -> Settings {
        Settings {
            trigger: front.trigger.map_or(self.trigger, trigger),
            case: front.case.map_or(self.case, case),
            whole_word: front.word.unwrap_or(self.whole_word),
            keep_delimiter: front.keep_delimiter.unwrap_or(self.keep_delimiter),
            delimiters: self.delimiters.clone(),
            scope: self.scope.clone(),
            enabled: self.enabled && front.enabled.unwrap_or(true),
        }
    }
}

fn trigger(mode: aralo_snippet::TriggerMode) -> Trigger {
    match mode {
        aralo_snippet::TriggerMode::Immediate => Trigger::Immediate,
        aralo_snippet::TriggerMode::Delimiter => Trigger::Delimiter,
    }
}

fn case(mode: aralo_snippet::CaseMode) -> CaseMode {
    match mode {
        aralo_snippet::CaseMode::Exact => CaseMode::Exact,
        aralo_snippet::CaseMode::Ignore => CaseMode::Ignore,
        aralo_snippet::CaseMode::Adaptive => CaseMode::Adaptive,
    }
}
