use serde::{Deserialize, Serialize};

use crate::{CaseMode, Extra, ParseError, TriggerMode};

/// The optional file that describes a group folder.
pub const GROUP_FILE_NAME: &str = "_group.yaml";

/// `_group.yaml`. Every key is optional; a folder without the file is a group
/// named after the folder that inherits everything.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GroupFile {
    /// Display name; the folder name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// `false` switches off this group and every group inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Apps the group works in. Absent means "inherit"; `{}` means everywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeSpec>,
    #[serde(default, skip_serializing_if = "Defaults::is_empty")]
    pub defaults: Defaults,
    #[serde(flatten)]
    pub extra: Extra,
}

/// App scope: bundle IDs on macOS, executable names on Windows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScopeSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub except: Vec<String>,
}

/// Values the group's snippets and sub-groups inherit unless they set their own.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Defaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<TriggerMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case: Option<CaseMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_delimiter: Option<bool>,
    /// Every character of this string is a delimiter, e.g. `" \t\n.,;"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delimiters: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Defaults {
    fn is_empty(&self) -> bool {
        *self == Defaults::default()
    }
}

impl GroupFile {
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let group: GroupFile = serde_yaml_ng::from_str(text)?;
        if let Some(scope) = &group.scope {
            if !scope.only.is_empty() && !scope.except.is_empty() {
                return Err(ParseError::ConflictingScope);
            }
        }
        Ok(group)
    }

    pub fn to_file_string(&self) -> Result<String, ParseError> {
        Ok(serde_yaml_ng::to_string(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_group_file() {
        let group = GroupFile::parse(
            "name: Support\ncolour: \"#3478F6\"\nicon: lifepreserver\nenabled: true\n\
             scope:\n  only: [com.apple.mail, com.tinyspeck.slackmacgap]\n\
             defaults:\n  trigger: immediate\n  case: exact\n  word: false\n  delimiters: \" \\t.\"\n",
        )
        .unwrap();
        assert_eq!(group.name.as_deref(), Some("Support"));
        assert_eq!(group.scope.as_ref().unwrap().only.len(), 2);
        assert_eq!(group.defaults.trigger, Some(TriggerMode::Immediate));
        assert_eq!(group.defaults.case, Some(CaseMode::Exact));
        assert_eq!(group.defaults.word, Some(false));
        assert_eq!(group.defaults.delimiters.as_deref(), Some(" \t."));
    }

    #[test]
    fn an_empty_file_inherits_everything() {
        assert_eq!(GroupFile::parse("").unwrap(), GroupFile::default());
        assert_eq!(
            GroupFile::parse("\n# nothing yet\n").unwrap(),
            GroupFile::default()
        );
    }

    #[test]
    fn an_empty_scope_is_distinct_from_no_scope() {
        let group = GroupFile::parse("scope: {}\n").unwrap();
        assert_eq!(group.scope, Some(ScopeSpec::default()));
    }

    #[test]
    fn rejects_a_scope_with_both_lists() {
        assert!(matches!(
            GroupFile::parse("scope:\n  only: [a.b]\n  except: [c.d]\n"),
            Err(ParseError::ConflictingScope)
        ));
    }

    #[test]
    fn unknown_keys_survive_a_save() {
        let group =
            GroupFile::parse("name: X\nsubscription:\n  url: https://example.org\n").unwrap();
        let saved = group.to_file_string().unwrap();
        assert!(saved.contains("subscription"));
        assert_eq!(GroupFile::parse(&saved).unwrap(), group);
    }
}
