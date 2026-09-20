use serde::{Deserialize, Deserializer, Serialize};

use crate::{Extra, ParseError, SnippetId};

/// Snippet files are Markdown files, so any editor and any diff tool reads them.
pub const SNIPPET_EXTENSION: &str = "md";

const FENCE: &str = "---";

/// One snippet file: front matter plus body.
#[derive(Debug, Clone, PartialEq)]
pub struct SnippetFile {
    pub front: FrontMatter,
    /// The template text, exactly as written. The one newline that ends the
    /// file is not part of it.
    pub body: String,
}

/// The YAML block at the top of a snippet file. A key left out means "inherit
/// from the group"; see `docs/format/snippet.md`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FrontMatter {
    /// Missing in hand-written files; Aralo assigns one on the first save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<SnippetId>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// A single string is accepted on read; a list is always written.
    #[serde(
        default,
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub abbr: Vec<String>,
    #[serde(rename = "type", default, skip_serializing_if = "SnippetKind::is_text")]
    pub kind: SnippetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<TriggerMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case: Option<CaseMode>,
    /// Whole-word rule: expand only after a non-word character.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word: Option<bool>,
    /// Re-insert the delimiter that triggered the expansion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_delimiter: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<AiSettings>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnippetKind {
    #[default]
    Text,
    Rich,
    Command,
    Prompt,
    Script,
}

impl SnippetKind {
    fn is_text(&self) -> bool {
        *self == SnippetKind::Text
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriggerMode {
    Immediate,
    Delimiter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaseMode {
    Exact,
    Ignore,
    Adaptive,
}

/// What an AI block in this snippet may see and which profile runs it (PRD P4).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AiSettings {
    /// Context kinds the snippet declares. Nothing else may be sent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

fn one_or_many<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Option::<OneOrMany>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(OneOrMany::One(one)) => vec![one],
        Some(OneOrMany::Many(many)) => many,
    })
}

impl SnippetFile {
    /// Parses a snippet file. A UTF-8 byte order mark and CRLF line endings
    /// around the front matter are accepted.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let (first, mut rest) = split_line(text);
        if first != FENCE {
            return Err(ParseError::MissingFrontMatter);
        }
        let yaml_start = rest;
        let (yaml, body) = loop {
            if rest.is_empty() {
                return Err(ParseError::UnclosedFrontMatter);
            }
            let (line, after) = split_line(rest);
            if line == FENCE {
                let yaml_len = yaml_start.len() - rest.len();
                break (&yaml_start[..yaml_len], after);
            }
            rest = after;
        };

        let front: FrontMatter = if yaml.trim().is_empty() {
            FrontMatter::default()
        } else {
            serde_yaml_ng::from_str(yaml)?
        };
        let body = body
            .strip_suffix("\r\n")
            .or_else(|| body.strip_suffix('\n'))
            .unwrap_or(body);
        Ok(Self {
            front,
            body: body.to_owned(),
        })
    }

    /// The file contents to write. `parse(to_file_string())` gives `self` back.
    pub fn to_file_string(&self) -> Result<String, ParseError> {
        let yaml = serde_yaml_ng::to_string(&self.front)?;
        let mut out = String::with_capacity(yaml.len() + self.body.len() + 10);
        out.push_str(FENCE);
        out.push('\n');
        // An empty mapping serialises as `{}`; an empty block reads better.
        if yaml.trim() != "{}" {
            out.push_str(&yaml);
            if !yaml.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str(FENCE);
        out.push('\n');
        out.push_str(&self.body);
        out.push('\n');
        Ok(out)
    }
}

/// Splits off the first line, without its `\n` or `\r\n`.
fn split_line(text: &str) -> (&str, &str) {
    match text.split_once('\n') {
        Some((line, rest)) => (line.strip_suffix('\r').unwrap_or(line), rest),
        None => (text, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = "\
---
id: 01J8ZK3V5Q8W6T9X2N4R7M0ABC
label: Enterprise refund reply
abbr: [\";refund\", \";rf\"]
type: text              # text | rich | command | prompt | script
trigger: delimiter
case: adaptive
word: true
tags: [support, billing]
ai:
  context: [fillins, selection]
  profile: default
---
Hi {{field: name}},

{{snippet: refund-policy}}
";

    #[test]
    fn parses_the_example_from_the_plan() {
        let file = SnippetFile::parse(EXAMPLE).unwrap();
        let front = &file.front;
        assert_eq!(front.id.unwrap().to_string(), "01J8ZK3V5Q8W6T9X2N4R7M0ABC");
        assert_eq!(front.label, "Enterprise refund reply");
        assert_eq!(front.abbr, [";refund", ";rf"]);
        assert_eq!(front.kind, SnippetKind::Text);
        assert_eq!(front.trigger, Some(TriggerMode::Delimiter));
        assert_eq!(front.case, Some(CaseMode::Adaptive));
        assert_eq!(front.word, Some(true));
        assert_eq!(front.keep_delimiter, None);
        assert_eq!(front.tags, ["support", "billing"]);
        let ai = front.ai.as_ref().unwrap();
        assert_eq!(ai.context, ["fillins", "selection"]);
        assert_eq!(ai.profile.as_deref(), Some("default"));
        assert_eq!(
            file.body,
            "Hi {{field: name}},\n\n{{snippet: refund-policy}}"
        );
    }

    #[test]
    fn a_minimal_hand_written_file_is_enough() {
        let file = SnippetFile::parse("---\nabbr: ;sig\n---\nBest,\nSam").unwrap();
        assert_eq!(file.front.id, None);
        assert_eq!(file.front.abbr, [";sig"]);
        assert_eq!(file.body, "Best,\nSam");
    }

    #[test]
    fn unknown_keys_survive_a_save() {
        let text =
            "---\nabbr: [x]\nfuture_key:\n  nested: [1, 2]\nai:\n  temperature: 0.2\n---\nbody\n";
        let file = SnippetFile::parse(text).unwrap();
        assert!(file.front.extra.contains_key("future_key"));
        let saved = file.to_file_string().unwrap();
        let again = SnippetFile::parse(&saved).unwrap();
        assert_eq!(again, file);
        assert!(saved.contains("future_key"));
        assert!(saved.contains("temperature"));
    }

    #[test]
    fn the_body_round_trips_byte_for_byte() {
        for body in [
            "",
            "one line",
            "trailing blank line\n",
            "\n\nleading blank lines",
            "---\nnot a fence in the body\n---",
            "  indented\n\ttabbed  ",
            "crlf\r\nkept\r\n",
        ] {
            let file = SnippetFile {
                front: FrontMatter {
                    abbr: vec!["x".into()],
                    ..FrontMatter::default()
                },
                body: body.to_owned(),
            };
            let again = SnippetFile::parse(&file.to_file_string().unwrap()).unwrap();
            assert_eq!(again.body, body, "body {body:?}");
        }
    }

    #[test]
    fn empty_front_matter_is_valid_and_writes_back_empty() {
        let file = SnippetFile::parse("---\n---\nhello\n").unwrap();
        assert_eq!(file.front, FrontMatter::default());
        assert_eq!(file.to_file_string().unwrap(), "---\n---\nhello\n");
    }

    #[test]
    fn accepts_a_byte_order_mark_and_crlf_fences() {
        let file = SnippetFile::parse("\u{feff}---\r\nabbr: [x]\r\n---\r\nbody\r\n").unwrap();
        assert_eq!(file.front.abbr, ["x"]);
        assert_eq!(file.body, "body");
    }

    #[test]
    fn reports_missing_and_unclosed_front_matter() {
        assert!(matches!(
            SnippetFile::parse("# Just a readme\n"),
            Err(ParseError::MissingFrontMatter)
        ));
        assert!(matches!(
            SnippetFile::parse("---\nabbr: [x]\nbody without a closing fence\n"),
            Err(ParseError::UnclosedFrontMatter)
        ));
        assert!(matches!(
            SnippetFile::parse("---\nabbr: [unclosed\n---\nbody\n"),
            Err(ParseError::Yaml(_))
        ));
        assert!(matches!(
            SnippetFile::parse("---\ntrigger: sometimes\n---\nbody\n"),
            Err(ParseError::Yaml(_))
        ));
    }
}
