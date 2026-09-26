//! The compatibility table: how text goes into each app.
//!
//! Apps disagree about synthetic keys and about paste, so the insert method,
//! the delays, the undo style and the delete strategy are data, not code:
//! `data/compat/apps.toml`, compiled in here. The core looks the front app up
//! when it changes and sends the [`InjectionProfile`] along with every plan.
//! The shell follows it and decides nothing.

use serde::Deserialize;

/// The table that ships inside this build.
pub const BUNDLED: &str = include_str!("../../../data/compat/apps.toml");

/// The newest table format this build reads.
pub const FORMAT_VERSION: u32 = 0;

/// Upper bounds. The table will also arrive as a signed download, and a bad
/// number in it must not be able to stall the injector.
pub const MAX_TYPING_LIMIT: u32 = 10_000;
pub const MAX_KEY_DELAY_MS: u32 = 250;
pub const MAX_PASTE_SETTLE_MS: u32 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InsertChoice {
    /// Type short single-line text, paste the rest.
    Auto,
    /// Always type. A line break is a Return key.
    Type,
    /// Always paste.
    Paste,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UndoStyle {
    /// A pasted expansion is one undo step in the app: forward one Cmd+Z.
    Native,
    /// Take every expansion back with Backspace.
    Backspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeleteStrategy {
    /// One Backspace per character.
    Backspace,
    /// Shift+Left over the characters, then one Backspace.
    Select,
}

/// How the shell inserts, deletes and undoes in one app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectionProfile {
    pub insert: InsertChoice,
    /// The longest text [`InsertChoice::Auto`] still types, in UTF-16 units.
    pub typing_limit: u32,
    /// Pause after every synthetic key.
    pub key_delay_ms: u32,
    /// How long the app gets to read the pasteboard before it is restored.
    pub paste_settle_ms: u32,
    pub undo: UndoStyle,
    pub delete: DeleteStrategy,
}

impl Default for InjectionProfile {
    /// What a table with no `[defaults]` means. `data/compat/apps.toml` spells
    /// the same values out, and a test keeps the two in step.
    fn default() -> Self {
        Self {
            insert: InsertChoice::Auto,
            typing_limit: 120,
            key_delay_ms: 1,
            paste_settle_ms: 250,
            undo: UndoStyle::Native,
            delete: DeleteStrategy::Backspace,
        }
    }
}

/// One app the table knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    pub name: String,
    pub bundle_id: String,
    pub profile: InjectionProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompatError {
    #[error("not a compatibility table: {0}")]
    Syntax(String),
    #[error("the table is format version {found}, and this Aralo reads up to version {supported}")]
    NewerVersion { found: u32, supported: u32 },
    #[error("app entry {position} has an empty bundle_id")]
    EmptyBundleId { position: usize },
    #[error("{bundle_id} is listed twice")]
    Duplicate { bundle_id: String },
    #[error("{field} = {value} in {owner} is too large; the most allowed is {max}")]
    OutOfRange {
        owner: String,
        field: &'static str,
        value: u32,
        max: u32,
    },
}

/// The parsed table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompatTable {
    revision: u32,
    defaults: InjectionProfile,
    apps: Vec<AppEntry>,
}

impl CompatTable {
    /// The table compiled into this build. A test proves it parses; if it ever
    /// did not, every app would get the built-in defaults rather than a panic
    /// in the process that holds the event tap.
    pub fn bundled() -> Self {
        Self::parse(BUNDLED).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Result<Self, CompatError> {
        let file: File =
            toml::from_str(text).map_err(|error| CompatError::Syntax(error.to_string()))?;
        if file.version > FORMAT_VERSION {
            return Err(CompatError::NewerVersion {
                found: file.version,
                supported: FORMAT_VERSION,
            });
        }
        let defaults = file
            .defaults
            .resolve("defaults", InjectionProfile::default())?;
        let mut apps: Vec<AppEntry> = Vec::with_capacity(file.app.len());
        for (index, app) in file.app.into_iter().enumerate() {
            if app.bundle_id.trim().is_empty() {
                return Err(CompatError::EmptyBundleId {
                    position: index + 1,
                });
            }
            if apps
                .iter()
                .any(|known| known.bundle_id.eq_ignore_ascii_case(&app.bundle_id))
            {
                return Err(CompatError::Duplicate {
                    bundle_id: app.bundle_id,
                });
            }
            let overrides = Overrides {
                insert: app.insert,
                typing_limit: app.typing_limit,
                key_delay_ms: app.key_delay_ms,
                paste_settle_ms: app.paste_settle_ms,
                undo: app.undo,
                delete: app.delete,
            };
            let profile = overrides.resolve(&app.bundle_id, defaults)?;
            apps.push(AppEntry {
                name: app.name.unwrap_or_default(),
                bundle_id: app.bundle_id,
                profile,
            });
        }
        Ok(Self {
            revision: file.revision,
            defaults,
            apps,
        })
    }

    /// Which edition of the table this is. Every change to
    /// `data/compat/apps.toml` raises it by one. A downloaded table replaces
    /// the one in use only when its revision is higher, so an older signed
    /// table cannot be played back over a newer one ([`crate::data_update`]).
    /// A table that leaves it out is revision 0.
    pub fn revision(&self) -> u32 {
        self.revision
    }

    /// The profile for an app, or the defaults for one the table does not
    /// know. Bundle IDs compare without ASCII case, as the engine's scopes do.
    pub fn profile_for(&self, bundle_id: &str) -> InjectionProfile {
        self.apps
            .iter()
            .find(|app| app.bundle_id.eq_ignore_ascii_case(bundle_id))
            .map_or(self.defaults, |app| app.profile)
    }

    pub fn defaults(&self) -> InjectionProfile {
        self.defaults
    }

    /// Every app the table lists, in file order. The injection matrix runs
    /// over these.
    pub fn apps(&self) -> &[AppEntry] {
        &self.apps
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    #[serde(default)]
    revision: u32,
    #[serde(default)]
    defaults: Overrides,
    #[serde(default)]
    app: Vec<App>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct App {
    name: Option<String>,
    bundle_id: String,
    // Spelled out, not a flattened `Overrides`: serde cannot refuse unknown
    // keys through `flatten`, and a typo here must not pass in silence.
    insert: Option<InsertChoice>,
    typing_limit: Option<u32>,
    key_delay_ms: Option<u32>,
    paste_settle_ms: Option<u32>,
    undo: Option<UndoStyle>,
    delete: Option<DeleteStrategy>,
}

/// The settings an entry may give. What it leaves out comes from `[defaults]`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Overrides {
    insert: Option<InsertChoice>,
    typing_limit: Option<u32>,
    key_delay_ms: Option<u32>,
    paste_settle_ms: Option<u32>,
    undo: Option<UndoStyle>,
    delete: Option<DeleteStrategy>,
}

impl Overrides {
    fn resolve(self, owner: &str, base: InjectionProfile) -> Result<InjectionProfile, CompatError> {
        let bounded = |field: &'static str, value: Option<u32>, fallback: u32, max: u32| {
            let value = value.unwrap_or(fallback);
            if value > max {
                return Err(CompatError::OutOfRange {
                    owner: owner.to_owned(),
                    field,
                    value,
                    max,
                });
            }
            Ok(value)
        };
        Ok(InjectionProfile {
            insert: self.insert.unwrap_or(base.insert),
            typing_limit: bounded(
                "typing_limit",
                self.typing_limit,
                base.typing_limit,
                MAX_TYPING_LIMIT,
            )?,
            key_delay_ms: bounded(
                "key_delay_ms",
                self.key_delay_ms,
                base.key_delay_ms,
                MAX_KEY_DELAY_MS,
            )?,
            paste_settle_ms: bounded(
                "paste_settle_ms",
                self.paste_settle_ms,
                base.paste_settle_ms,
                MAX_PASTE_SETTLE_MS,
            )?,
            undo: self.undo.unwrap_or(base.undo),
            delete: self.delete.unwrap_or(base.delete),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_table_parses_and_lists_the_matrix_apps() {
        let table = CompatTable::parse(BUNDLED).expect("data/compat/apps.toml must parse");
        assert_eq!(table.apps().len(), 15);
        assert!(table.apps().iter().all(|app| !app.name.is_empty()));
    }

    #[test]
    fn the_bundled_defaults_are_the_built_in_defaults() {
        let table = CompatTable::parse(BUNDLED).unwrap();
        assert_eq!(table.defaults(), InjectionProfile::default());
    }

    #[test]
    fn terminals_are_typed_into_and_undone_with_backspace() {
        let table = CompatTable::bundled();
        for bundle_id in ["com.apple.Terminal", "com.googlecode.iterm2"] {
            let profile = table.profile_for(bundle_id);
            assert_eq!(profile.insert, InsertChoice::Type, "{bundle_id}");
            assert_eq!(profile.undo, UndoStyle::Backspace, "{bundle_id}");
        }
    }

    #[test]
    fn an_unknown_app_gets_the_defaults() {
        let table = CompatTable::bundled();
        assert_eq!(
            table.profile_for("com.example.unheard-of"),
            table.defaults()
        );
        assert_eq!(table.profile_for(""), table.defaults());
    }

    #[test]
    fn bundle_ids_match_without_ascii_case() {
        let table = CompatTable::bundled();
        assert_eq!(
            table.profile_for("COM.APPLE.TERMINAL"),
            table.profile_for("com.apple.Terminal")
        );
    }

    #[test]
    fn an_entry_overrides_only_what_it_names() {
        let table = CompatTable::parse(
            r#"
            version = 0
            [defaults]
            key_delay_ms = 4
            [[app]]
            bundle_id = "com.example.slow"
            paste_settle_ms = 900
            delete = "select"
            "#,
        )
        .unwrap();
        let profile = table.profile_for("com.example.slow");
        assert_eq!(profile.key_delay_ms, 4);
        assert_eq!(profile.paste_settle_ms, 900);
        assert_eq!(profile.delete, DeleteStrategy::Select);
        assert_eq!(profile.insert, InsertChoice::Auto);
        assert_eq!(table.defaults().paste_settle_ms, 250);
    }

    #[test]
    fn a_table_with_only_a_version_is_all_defaults() {
        let table = CompatTable::parse("version = 0").unwrap();
        assert_eq!(table, CompatTable::default());
    }

    #[test]
    fn a_newer_format_is_refused() {
        assert_eq!(
            CompatTable::parse("version = 1"),
            Err(CompatError::NewerVersion {
                found: 1,
                supported: FORMAT_VERSION
            })
        );
    }

    #[test]
    fn a_delay_that_would_stall_the_injector_is_refused() {
        let error = CompatTable::parse(
            "version = 0\n[[app]]\nbundle_id = \"com.example.app\"\nkey_delay_ms = 60000\n",
        )
        .unwrap_err();
        assert_eq!(
            error,
            CompatError::OutOfRange {
                owner: "com.example.app".to_owned(),
                field: "key_delay_ms",
                value: 60_000,
                max: MAX_KEY_DELAY_MS
            }
        );
    }

    #[test]
    fn typos_and_duplicates_are_refused() {
        let unknown_key = "version = 0\n[[app]]\nbundle_id = \"a.b\"\ninsrt = \"type\"\n";
        assert!(matches!(
            CompatTable::parse(unknown_key),
            Err(CompatError::Syntax(_))
        ));
        let unknown_value = "version = 0\n[defaults]\ninsert = \"telepathy\"\n";
        assert!(matches!(
            CompatTable::parse(unknown_value),
            Err(CompatError::Syntax(_))
        ));
        let twice = "version = 0\n[[app]]\nbundle_id = \"a.b\"\n[[app]]\nbundle_id = \"A.B\"\n";
        assert_eq!(
            CompatTable::parse(twice),
            Err(CompatError::Duplicate {
                bundle_id: "A.B".to_owned()
            })
        );
        let empty = "version = 0\n[[app]]\nbundle_id = \" \"\n";
        assert_eq!(
            CompatTable::parse(empty),
            Err(CompatError::EmptyBundleId { position: 1 })
        );
    }
}
