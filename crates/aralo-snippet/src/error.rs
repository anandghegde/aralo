/// Why a library file could not be read. Messages never quote file contents
/// beyond what the YAML parser reports about the offending key.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("the file does not start with a `---` front matter block")]
    MissingFrontMatter,
    #[error("the front matter block is not closed by a `---` line")]
    UnclosedFrontMatter,
    #[error("invalid YAML: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),
    #[error("`scope` sets both `only` and `except`; use one")]
    ConflictingScope,
    #[error("format version {found} is newer than this build understands ({supported})")]
    UnsupportedFormat { found: u32, supported: u32 },
}
