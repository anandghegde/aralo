//! Where Aralo keeps what it can throw away.
//!
//! The library is the user's folder and the truth ([ADR-0006]). Everything in
//! here is derived from it: the search index, and later the vectors and the
//! merge bases. Deleting this folder costs a rebuild, never a snippet, which is
//! why it may live somewhere the user never looks
//! ([ADR-0011](../../../docs/adr/0011-shell-defaults.md)).
//!
//! Usage statistics are the exception: they are not rebuildable, because
//! nothing else records that a snippet was expanded. They are local, never
//! synced, and losing them costs a sorted list its order.
//!
//! [ADR-0006]: ../../../docs/adr/0006-files-as-source-of-truth.md

use std::path::{Path, PathBuf};

/// How much of the hash goes in a file name. 64 bits of blake3: two libraries
/// would have to be chosen adversarially to collide, and the cost of a
/// collision is one rebuilt cache.
const HASH_CHARS: usize = 16;

/// The folder Aralo keeps its caches in, or `None` when the machine will not
/// say where the user's home is.
///
/// `ARALO_STATE` overrides it, which is how the test harness keeps a run from
/// touching the real one.
pub fn state_folder() -> Option<PathBuf> {
    if let Some(from_environment) = std::env::var_os("ARALO_STATE") {
        if !from_environment.is_empty() {
            return Some(PathBuf::from(from_environment));
        }
    }
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Aralo"),
    )
}

/// Where the search index for the library at `root` belongs.
///
/// The name carries the folder's own name, so that a person looking at the
/// cache folder can tell which library a file belongs to, and a hash of the
/// whole path, so that two libraries called `Aralo` in two places do not share
/// one index. It is derived, not stored: the same library always resolves to
/// the same file, and a library that moves gets a new one and rebuilds.
pub fn index_path(root: &Path) -> Option<PathBuf> {
    Some(index_in(&state_folder()?, root))
}

/// The same, in a cache folder the caller names. A shell knows where its
/// platform puts caches; the core only decides what the file is called.
pub fn index_in(cache: &Path, root: &Path) -> PathBuf {
    let canonical = root.canonicalize();
    let path = canonical.as_deref().unwrap_or(root);
    let hash = blake3::hash(path.as_os_str().as_encoded_bytes()).to_hex();
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let name = sanitise(&name);
    cache
        .join("index")
        .join(format!("{name}-{}.sqlite3", &hash[..HASH_CHARS]))
}

/// A folder name a file system will take: letters, digits and hyphens, and
/// never so long that the path around it becomes the problem.
fn sanitise(name: &str) -> String {
    let mut out = String::new();
    for character in name.chars().take(32) {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() {
        "library".to_owned()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_is_not_a_file_name_becomes_one() {
        assert_eq!(sanitise("Aralo"), "aralo");
        assert_eq!(sanitise("My Snippets/2024"), "my-snippets-2024");
        assert_eq!(sanitise("../.."), "library");
        assert_eq!(sanitise(""), "library");
        assert!(sanitise(&"x".repeat(80)).len() <= 32);
    }

    #[test]
    fn two_libraries_of_one_name_get_two_indexes() {
        let folder = tempfile::tempdir().unwrap();
        let first = folder.path().join("one/Aralo");
        let second = folder.path().join("two/Aralo");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();

        let first = index_path(&first).expect("a home to put it in");
        let second = index_path(&second).expect("a home to put it in");
        assert_ne!(first, second);
        assert!(first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("aralo-"));
        assert_eq!(first.parent(), second.parent());
    }
}
