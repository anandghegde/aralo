use serde::{Deserialize, Serialize};

use crate::{Extra, ParseError};

/// The file at the library root that marks a folder as an Aralo library.
pub const MANIFEST_FILE_NAME: &str = "aralo.yaml";

/// The library format this build reads and writes. Version 0 is the
/// pre-release format and may still change without a migration.
pub const FORMAT_VERSION: u32 = 0;

/// `aralo.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    /// A ULID that names this library in caches kept outside the folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Manifest {
    pub fn new(name: Option<String>) -> Self {
        Self {
            format: FORMAT_VERSION,
            id: Some(ulid::Ulid::generate().to_string()),
            name,
            extra: Extra::new(),
        }
    }

    /// Parses the manifest and refuses a format newer than this build knows,
    /// so an old Aralo never rewrites a library it only half understands.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let manifest: Manifest = serde_yaml_ng::from_str(text)?;
        if manifest.format > FORMAT_VERSION {
            return Err(ParseError::UnsupportedFormat {
                found: manifest.format,
                supported: FORMAT_VERSION,
            });
        }
        Ok(manifest)
    }

    pub fn to_file_string(&self) -> Result<String, ParseError> {
        Ok(serde_yaml_ng::to_string(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_manifest_round_trips() {
        let manifest = Manifest::new(Some("My Library".into()));
        let again = Manifest::parse(&manifest.to_file_string().unwrap()).unwrap();
        assert_eq!(again, manifest);
        assert_eq!(again.format, FORMAT_VERSION);
    }

    #[test]
    fn refuses_a_newer_format() {
        let text = format!("format: {}\n", FORMAT_VERSION + 1);
        assert!(matches!(
            Manifest::parse(&text),
            Err(ParseError::UnsupportedFormat { .. })
        ));
    }
}
