//! Context assembly: the second stage. A request names the context kinds its
//! snippet or command declared, and only those are asked for. Undeclared
//! context is never requested from the shell, not merely never sent (PRD P4).

/// The context a snippet may declare in `ai.context`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContextKind {
    /// The answers to the snippet's form.
    Fillins,
    /// The text selected in the app the user is in.
    Selection,
    Clipboard,
    /// The name of the app the user is in.
    App,
    /// The title of its front window.
    Window,
}

impl ContextKind {
    pub const ALL: [Self; 5] = [
        Self::Fillins,
        Self::Selection,
        Self::Clipboard,
        Self::App,
        Self::Window,
    ];

    /// The name as a snippet file spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fillins => "fillins",
            Self::Selection => "selection",
            Self::Clipboard => "clipboard",
            Self::App => "app",
            Self::Window => "window",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == name.trim())
    }

    /// Reads a snippet's `ai.context` list. Names Aralo does not know are
    /// returned apart, so the editor can mark them; they grant nothing.
    pub fn parse_declared<S: AsRef<str>>(names: &[S]) -> (Vec<Self>, Vec<String>) {
        let mut kinds = Vec::new();
        let mut unknown = Vec::new();
        for name in names {
            match Self::parse(name.as_ref()) {
                Some(kind) if !kinds.contains(&kind) => kinds.push(kind),
                Some(_) => {}
                None => unknown.push(name.as_ref().to_owned()),
            }
        }
        (kinds, unknown)
    }
}

/// Where context comes from: the shell, or a session that already holds the
/// form's answers. The gateway calls it once for each declared kind and for
/// nothing else.
pub trait ContextSource {
    /// The text for one kind, or `None` when there is none to give, such as
    /// no selection.
    fn fetch(&mut self, kind: ContextKind) -> Option<String>;
}

/// No context at all, for a request that declares none.
impl ContextSource for () {
    fn fetch(&mut self, _kind: ContextKind) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextItem {
    pub kind: ContextKind,
    pub text: String,
}

/// One line of what the preview panel shows before a request is sent: which
/// kind, and how much of it. The text itself is not in the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestEntry {
    pub kind: ContextKind,
    /// `None` when the kind was declared but the shell had nothing for it.
    pub bytes: Option<usize>,
}

/// Asks the source for each declared kind, once, in the order declared.
pub fn assemble(
    declared: &[ContextKind],
    source: &mut dyn ContextSource,
) -> (Vec<ContextItem>, Vec<ManifestEntry>) {
    let mut items = Vec::new();
    let mut manifest = Vec::new();
    for &kind in declared {
        if manifest
            .iter()
            .any(|entry: &ManifestEntry| entry.kind == kind)
        {
            continue;
        }
        let text = source.fetch(kind);
        manifest.push(ManifestEntry {
            kind,
            bytes: text.as_ref().map(String::len),
        });
        if let Some(text) = text {
            items.push(ContextItem { kind, text });
        }
    }
    (items, manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_names_are_read_once_and_unknown_ones_kept_apart() {
        let (kinds, unknown) =
            ContextKind::parse_declared(&["selection", " app", "selection", "screen"]);
        assert_eq!(kinds, [ContextKind::Selection, ContextKind::App]);
        assert_eq!(unknown, ["screen"]);
    }
}
