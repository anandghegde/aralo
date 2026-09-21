//! The SQLite index: a cache of what is in the library folder.
//!
//! The files are the truth ([ADR-0006]); everything in `snippets`, `groups` and
//! `snippets_fts` is derived from them and can be thrown away and rebuilt. Two
//! tables are not: `stats` counts expansions and `vectors` holds embeddings that
//! cost real time to compute, so neither is touched by a rebuild and neither is
//! ever synced between machines.
//!
//! Expansion does not read the index. Abbreviations reach the engine through
//! [`Library::snapshot`], which needs only the loaded files, so a cold rebuild
//! of a large library is a background job that nothing waits for.
//!
//! [`Index::sync`] is the usual path: it compares each snippet's content hash,
//! path, group and enabled flag against the row already stored and writes only
//! what differs. [`Index::rebuild`] clears the derived tables and writes
//! everything. They must agree, which is what [`Index::rows`] is for.
//!
//! The index belongs in the application's own folder, never inside the library:
//! a SQLite file in a synced folder is a corruption waiting to happen.
//!
//! [ADR-0006]: ../../../docs/adr/0006-files-as-source-of-truth.md

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aralo_snippet::{SnippetId, SnippetKind};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::{Library, LoadedSnippet};

/// Bumped whenever a derived table changes shape. An index written by an older
/// Aralo is dropped and rebuilt rather than migrated: it is a cache.
const SCHEMA_VERSION: i64 = 2;

/// Column weights for `bm25`, in the order the FTS table declares them. An
/// abbreviation is what the user types, so it outranks a label, and both
/// outrank a word buried in a body.
const BM25_WEIGHTS: &str = "0.0, 8.0, 12.0, 4.0, 1.0";

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("cannot create the folder for the index at {path}: {source}")]
    Folder {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot open the index at {path}: {source}")]
    Open {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error("the index failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("the index holds `{0}`, which is not a snippet id")]
    NotAnId(String),
}

/// What one [`Index::sync`] or [`Index::rebuild`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Indexed {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    /// Snippets whose row was already right. A rebuild reports none: it writes
    /// every row whether it had to or not.
    pub unchanged: usize,
}

impl Indexed {
    pub fn changed(&self) -> bool {
        self.added + self.updated + self.removed > 0
    }
}

/// How often a snippet has been expanded on this machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub expansions: u64,
    pub last_used: Option<SystemTime>,
}

/// One row of the recents list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recent {
    pub id: SnippetId,
    pub expansions: u64,
    pub last_used: SystemTime,
}

/// The index for one library folder.
#[derive(Debug)]
pub struct Index {
    connection: Connection,
}

impl Index {
    /// Opens or creates the index at `path`, creating the folder above it.
    pub fn open(path: &Path) -> Result<Self, IndexError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|source| IndexError::Folder {
                path: parent.to_owned(),
                source,
            })?;
        }
        let connection = Connection::open(path).map_err(|source| IndexError::Open {
            path: path.to_owned(),
            source,
        })?;
        Self::prepare(connection)
    }

    /// An index that lasts as long as the value. Tests use it.
    pub fn open_in_memory() -> Result<Self, IndexError> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(connection: Connection) -> Result<Self, IndexError> {
        // The menu bar reads while the indexer writes, so readers must not
        // block. The index is a cache and the files are the truth, so a crash
        // that loses the last few writes costs a rebuild, not data.
        let _: String = connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        connection.execute_batch("PRAGMA synchronous = NORMAL;")?;

        let mut index = Self { connection };
        let version: i64 = index
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            index.reset()?;
        }
        Ok(index)
    }

    /// Drops the derived tables and recreates the schema. `stats` and `vectors`
    /// are left alone, so an upgrade does not cost the user their recents or an
    /// afternoon of embedding. Changing the shape of either of those two needs
    /// a real migration written here, because `IF NOT EXISTS` would quietly
    /// keep the old one.
    fn reset(&mut self) -> Result<(), IndexError> {
        self.connection.execute_batch(&format!(
            "BEGIN;
             DROP TABLE IF EXISTS snippets_fts;
             DROP TABLE IF EXISTS snippets;
             DROP TABLE IF EXISTS groups;

             -- One row per loaded snippet. `hash` covers the content only, so a
             -- snippet that moves folders keeps it, and with it its embedding.
             CREATE TABLE snippets (
                 id         TEXT PRIMARY KEY,
                 path       TEXT NOT NULL UNIQUE,
                 group_path TEXT NOT NULL,
                 hash       TEXT NOT NULL,
                 kind       TEXT NOT NULL,
                 label      TEXT NOT NULL,
                 abbr       TEXT NOT NULL,
                 tags       TEXT NOT NULL,
                 enabled    INTEGER NOT NULL,
                 body       TEXT NOT NULL
             ) STRICT;
             CREATE INDEX snippets_by_group ON snippets (group_path);

             -- The folder tree, with what each folder's `_group.yaml` says about
             -- itself. `enabled` is the resolved value: off is sticky, so a
             -- group inside a group that is off is off here too.
             CREATE TABLE groups (
                 path     TEXT PRIMARY KEY,
                 name     TEXT NOT NULL,
                 parent   TEXT NOT NULL,
                 depth    INTEGER NOT NULL,
                 snippets INTEGER NOT NULL,
                 colour   TEXT,
                 icon     TEXT,
                 enabled  INTEGER NOT NULL
             ) STRICT;

             -- Not an external-content table: the rows are small and written
             -- beside `snippets` anyway, and this keeps the two independent.
             CREATE VIRTUAL TABLE snippets_fts USING fts5(
                 id UNINDEXED,
                 label,
                 abbr,
                 tags,
                 body,
                 tokenize = 'unicode61 remove_diacritics 2'
             );

             -- Local only, never synced, never rebuilt (plan section 4.1).
             CREATE TABLE IF NOT EXISTS stats (
                 id         TEXT PRIMARY KEY,
                 expansions INTEGER NOT NULL,
                 last_used  INTEGER
             ) STRICT;

             -- Reserved for the embedding worker in M4. Keyed by content hash
             -- and model so it survives a rebuild, a rename and a move.
             CREATE TABLE IF NOT EXISTS vectors (
                 hash   TEXT NOT NULL,
                 model  TEXT NOT NULL,
                 vector BLOB NOT NULL,
                 PRIMARY KEY (hash, model)
             ) STRICT;

             PRAGMA user_version = {SCHEMA_VERSION};
             COMMIT;"
        ))?;
        Ok(())
    }

    /// Writes every snippet, whether it changed or not, after clearing what was
    /// there. Use it when the index is new or suspect; [`Index::sync`] is the
    /// one to reach for otherwise.
    pub fn rebuild(&mut self, library: &Library) -> Result<Indexed, IndexError> {
        let transaction = self.connection.transaction()?;
        transaction
            .execute_batch("DELETE FROM snippets_fts; DELETE FROM snippets; DELETE FROM groups;")?;
        for snippet in library.snippets() {
            put(&transaction, snippet)?;
        }
        write_groups(&transaction, library)?;
        let counted = Indexed {
            added: library.snippets().len(),
            ..Indexed::default()
        };
        transaction.commit()?;
        Ok(counted)
    }

    /// Writes only what differs from what is already stored, and drops the rows
    /// of snippets that are no longer in the library.
    pub fn sync(&mut self, library: &Library) -> Result<Indexed, IndexError> {
        let transaction = self.connection.transaction()?;
        let mut known = stored(&transaction)?;
        let mut counted = Indexed::default();

        for snippet in library.snippets() {
            let fresh = Stored::of(snippet);
            match known.remove(&snippet.id) {
                Some(old) if old == fresh => counted.unchanged += 1,
                Some(_) => {
                    put(&transaction, snippet)?;
                    counted.updated += 1;
                }
                None => {
                    put(&transaction, snippet)?;
                    counted.added += 1;
                }
            }
        }

        // Whatever is left in `known` has gone from the folder.
        for id in known.keys() {
            let id = id.to_string();
            transaction
                .prepare_cached("DELETE FROM snippets WHERE id = ?1")?
                .execute(params![id])?;
            transaction
                .prepare_cached("DELETE FROM snippets_fts WHERE id = ?1")?
                .execute(params![id])?;
            counted.removed += 1;
        }

        // The group tree is a handful of rows derived from the snippets, so it
        // is cheaper to rewrite than to diff.
        write_groups(&transaction, library)?;
        transaction.commit()?;
        Ok(counted)
    }

    /// The best matches for `text`, most relevant first. This is the literal
    /// half of search; fusing it with the fuzzy matching in
    /// [`Searcher`](crate::Searcher) is task 2.5.
    ///
    /// The text is treated as words to find, not as FTS5 syntax, so a user who
    /// types `NOT` or `"` gets results rather than an error. The last word
    /// matches as a prefix, which is what search-as-you-type needs.
    pub fn search(&self, text: &str, limit: usize) -> Result<Vec<SnippetId>, IndexError> {
        let Some(query) = fts_query(text) else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare_cached(&format!(
            "SELECT id FROM snippets_fts
             WHERE snippets_fts MATCH ?1
             ORDER BY bm25(snippets_fts, {BM25_WEIGHTS})
             LIMIT ?2"
        ))?;
        let rows =
            statement.query_map(params![query, count(limit)], |row| row.get::<_, String>(0))?;
        rows.map(|row| parse_id(&row?)).collect()
    }

    /// Counts one expansion of `id`. Nothing else writes `stats`.
    pub fn record_expansion(&self, id: SnippetId, at: SystemTime) -> Result<(), IndexError> {
        self.connection
            .prepare_cached(
                "INSERT INTO stats (id, expansions, last_used) VALUES (?1, 1, ?2)
                 ON CONFLICT(id) DO UPDATE SET
                     expansions = expansions + 1,
                     last_used = excluded.last_used",
            )?
            .execute(params![id.to_string(), millis(at)])?;
        Ok(())
    }

    pub fn stats(&self, id: SnippetId) -> Result<Stats, IndexError> {
        let stored = self
            .connection
            .prepare_cached("SELECT expansions, last_used FROM stats WHERE id = ?1")?
            .query_row(params![id.to_string()], |row| {
                Ok(Stats {
                    expansions: row.get::<_, i64>(0)?.max(0) as u64,
                    last_used: row.get::<_, Option<i64>>(1)?.map(time),
                })
            })
            .optional()?;
        Ok(stored.unwrap_or_default())
    }

    /// The most recently expanded snippets, newest first. A row can outlive its
    /// snippet: that is deliberate, so a snippet that comes back from the
    /// user's rubbish bin comes back with its history. Callers that show a list
    /// check each id against the library.
    pub fn recents(&self, limit: usize) -> Result<Vec<Recent>, IndexError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT id, expansions, last_used FROM stats
             WHERE last_used IS NOT NULL
             ORDER BY last_used DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(params![count(limit)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (id, expansions, last_used) = row?;
            Ok(Recent {
                id: parse_id(&id)?,
                expansions: expansions.max(0) as u64,
                last_used: time(last_used),
            })
        })
        .collect()
    }

    /// How many snippets are indexed.
    pub fn len(&self) -> Result<usize, IndexError> {
        let count: i64 = self
            .connection
            .query_row("SELECT count(*) FROM snippets", [], |row| row.get(0))?;
        Ok(count.max(0) as usize)
    }

    pub fn is_empty(&self) -> Result<bool, IndexError> {
        Ok(self.len()? == 0)
    }

    /// Every derived row as one ordered, printable line. Two indexes hold the
    /// same library exactly when these match, which is how a sync is checked
    /// against a rebuild. A large library makes a large dump; this is for
    /// tests and for `aralo index --dump`, not for the app.
    pub fn rows(&self) -> Result<Vec<String>, IndexError> {
        let mut lines = Vec::new();

        // `enabled` is the one column that is not text; casting it here keeps
        // the loop below uniform.
        let mut snippets = self.connection.prepare(
            "SELECT id, path, group_path, hash, kind, label, abbr, tags,
                    CAST(enabled AS TEXT), body
             FROM snippets ORDER BY id",
        )?;
        let mut rows = snippets.query([])?;
        while let Some(row) = rows.next()? {
            let mut line = String::from("snippet");
            for column in 0..10 {
                line.push('\t');
                line.push_str(&escape(&row.get::<_, String>(column)?));
            }
            lines.push(line);
        }

        let mut groups = self.connection.prepare(
            "SELECT path, name, parent, depth, snippets,
                    COALESCE(colour, ''), COALESCE(icon, ''), enabled
             FROM groups ORDER BY path",
        )?;
        let mut rows = groups.query([])?;
        while let Some(row) = rows.next()? {
            lines.push(format!(
                "group\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                escape(&row.get::<_, String>(0)?),
                escape(&row.get::<_, String>(1)?),
                escape(&row.get::<_, String>(2)?),
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                escape(&row.get::<_, String>(5)?),
                escape(&row.get::<_, String>(6)?),
                row.get::<_, i64>(7)?,
            ));
        }

        let mut fts = self
            .connection
            .prepare("SELECT id, label, abbr, tags, body FROM snippets_fts ORDER BY id")?;
        let mut rows = fts.query([])?;
        while let Some(row) = rows.next()? {
            lines.push(format!(
                "fts\t{}\t{}\t{}\t{}\t{}",
                escape(&row.get::<_, String>(0)?),
                escape(&row.get::<_, String>(1)?),
                escape(&row.get::<_, String>(2)?),
                escape(&row.get::<_, String>(3)?),
                escape(&row.get::<_, String>(4)?),
            ));
        }

        Ok(lines)
    }
}

/// The part of a row that decides whether a snippet needs rewriting. The hash
/// covers the content; these three are stored separately and change without it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stored {
    hash: String,
    path: String,
    group_path: String,
    enabled: bool,
}

impl Stored {
    fn of(snippet: &LoadedSnippet) -> Self {
        Self {
            hash: content_hash(snippet),
            path: portable(&snippet.path),
            group_path: snippet.group.join("/"),
            enabled: snippet.settings.enabled,
        }
    }
}

fn stored(transaction: &Transaction<'_>) -> Result<HashMap<SnippetId, Stored>, IndexError> {
    let mut statement =
        transaction.prepare("SELECT id, hash, path, group_path, enabled FROM snippets")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            Stored {
                hash: row.get(1)?,
                path: row.get(2)?,
                group_path: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
            },
        ))
    })?;
    let mut known = HashMap::new();
    for row in rows {
        let (id, stored) = row?;
        known.insert(parse_id(&id)?, stored);
    }
    Ok(known)
}

/// Writes one snippet's rows, replacing whatever held its id or its path. Both
/// matter: two snippets that swap files would otherwise collide on `path`, and
/// the row deleted by the collision would never come back.
fn put(transaction: &Transaction<'_>, snippet: &LoadedSnippet) -> Result<(), IndexError> {
    let row = Stored::of(snippet);
    let id = snippet.id.to_string();
    transaction
        .prepare_cached("DELETE FROM snippets WHERE id = ?1 OR path = ?2")?
        .execute(params![id, row.path])?;
    transaction
        .prepare_cached("DELETE FROM snippets_fts WHERE id = ?1")?
        .execute(params![id])?;

    let front = &snippet.file.front;
    let abbr = front.abbr.join("\n");
    let tags = front.tags.join("\n");
    transaction
        .prepare_cached(
            "INSERT INTO snippets
                 (id, path, group_path, hash, kind, label, abbr, tags, enabled, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?
        .execute(params![
            id,
            row.path,
            row.group_path,
            row.hash,
            kind_name(front.kind),
            front.label,
            abbr,
            tags,
            i64::from(row.enabled),
            snippet.file.body,
        ])?;
    transaction
        .prepare_cached(
            "INSERT INTO snippets_fts (id, label, abbr, tags, body) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(params![id, front.label, abbr, tags, snippet.file.body])?;
    Ok(())
}

fn write_groups(transaction: &Transaction<'_>, library: &Library) -> Result<(), IndexError> {
    // Every folder the loader walked, the root excepted: it is the library, not
    // a group anyone picks from a list.
    transaction.execute_batch("DELETE FROM groups;")?;
    for group in library.groups() {
        if group.path.is_empty() {
            continue;
        }
        let path = group.path.join("/");
        let parent = group.path[..group.path.len() - 1].join("/");
        transaction
            .prepare_cached(
                "INSERT INTO groups (path, name, parent, depth, snippets, colour, icon, enabled)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?
            .execute(params![
                path,
                group.name,
                parent,
                group.path.len() as i64,
                group.snippets as i64,
                group.colour,
                group.icon,
                i64::from(group.enabled),
            ])?;
    }
    Ok(())
}

/// A snippet's content, and nothing else: not its path, not its group, not the
/// enabled flag it inherits. Those live in their own columns and are compared
/// on their own, which leaves this hash stable across a rename or a move — the
/// property that lets `vectors` key on it.
fn content_hash(snippet: &LoadedSnippet) -> String {
    let mut hasher = blake3::Hasher::new();
    let front = &snippet.file.front;
    field(&mut hasher, kind_name(front.kind).as_bytes());
    field(&mut hasher, front.label.as_bytes());
    field(&mut hasher, &(front.abbr.len() as u64).to_le_bytes());
    for abbreviation in &front.abbr {
        field(&mut hasher, abbreviation.as_bytes());
    }
    field(&mut hasher, &(front.tags.len() as u64).to_le_bytes());
    for tag in &front.tags {
        field(&mut hasher, tag.as_bytes());
    }
    field(&mut hasher, snippet.file.body.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Length-prefixed, so that moving a character across a field boundary changes
/// the hash. Without it `label: ab` + `body: c` hashes the same as `label: a` +
/// `body: bc`.
fn field(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// The stored form of a kind. Spelled out rather than derived from `Debug`, so
/// that renaming a variant cannot silently invalidate every hash in the index;
/// the match is exhaustive, so adding one is a compile error here.
fn kind_name(kind: SnippetKind) -> &'static str {
    match kind {
        SnippetKind::Text => "text",
        SnippetKind::Rich => "rich",
        SnippetKind::Command => "command",
        SnippetKind::Prompt => "prompt",
        SnippetKind::Script => "script",
    }
}

/// `/` on every platform, so an index copied between machines still matches.
fn portable(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Turns what the user typed into an FTS5 query. Each word becomes a quoted
/// term, which strips `AND`, `NEAR`, `*` and the rest of their meaning, and the
/// last word matches as a prefix.
fn fts_query(text: &str) -> Option<String> {
    let mut terms: Vec<String> = text
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}\""))
        .collect();
    terms.last_mut()?.push('*');
    Some(terms.join(" "))
}

/// SQLite counts in `i64`. A `usize` that does not fit is not a real limit.
fn count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn parse_id(text: &str) -> Result<SnippetId, IndexError> {
    text.parse()
        .map_err(|_| IndexError::NotAnId(text.to_owned()))
}

/// Milliseconds since the epoch. A clock set before 1970 clamps to it rather
/// than failing: a wrong recents order is not worth refusing an expansion over.
fn millis(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn time(millis: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis.max(0) as u64)
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_the_user_typed_is_never_fts_syntax() {
        assert_eq!(fts_query("refund"), Some("\"refund\"*".to_owned()));
        assert_eq!(
            fts_query("best regards"),
            Some("\"best\" \"regards\"*".to_owned())
        );
        // None of these mean anything to FTS5 once quoted.
        assert_eq!(
            fts_query("a NOT b"),
            Some("\"a\" \"NOT\" \"b\"*".to_owned())
        );
        assert_eq!(fts_query("\"quoted\""), Some("\"quoted\"*".to_owned()));
        assert_eq!(fts_query("a*b^"), Some("\"a\" \"b\"*".to_owned()));
        assert_eq!(fts_query("   "), None);
        assert_eq!(fts_query(""), None);
    }

    #[test]
    fn a_path_is_stored_the_same_way_everywhere() {
        assert_eq!(
            portable(Path::new("Work/Email/note.md")),
            "Work/Email/note.md"
        );
        assert_eq!(portable(Path::new("note.md")), "note.md");
    }

    #[test]
    fn a_fresh_index_is_empty_and_has_the_current_schema() {
        let index = Index::open_in_memory().unwrap();
        assert!(index.is_empty().unwrap());
        let version: i64 = index
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn stats_start_at_nothing_and_count_up() {
        let index = Index::open_in_memory().unwrap();
        let id = SnippetId::generate();
        assert_eq!(index.stats(id).unwrap(), Stats::default());

        let at = UNIX_EPOCH + Duration::from_millis(1_700_000_000_000);
        index.record_expansion(id, at).unwrap();
        index
            .record_expansion(id, at + Duration::from_secs(1))
            .unwrap();

        let stats = index.stats(id).unwrap();
        assert_eq!(stats.expansions, 2);
        assert_eq!(stats.last_used, Some(at + Duration::from_secs(1)));

        let recents = index.recents(10).unwrap();
        assert_eq!(recents.len(), 1);
        assert_eq!(recents[0].id, id);
        assert_eq!(recents[0].expansions, 2);
    }

    #[test]
    fn a_clock_before_the_epoch_does_not_fail_an_expansion() {
        let index = Index::open_in_memory().unwrap();
        let id = SnippetId::generate();
        index
            .record_expansion(id, UNIX_EPOCH - Duration::from_secs(60))
            .unwrap();
        assert_eq!(index.stats(id).unwrap().expansions, 1);
    }
}
