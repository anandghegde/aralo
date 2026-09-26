//! Test helpers shared by the workspace's tests. Only ever a dev-dependency.
//!
//! [`tempdir`] is the one way a test makes a temporary folder. It behaves
//! exactly like `tempfile::tempdir()`, except that when `ARALO_KEEP_TEMP` is
//! set the folder is left on disk when the [`TempDir`] is dropped. The
//! key-leak scan (`scripts/key-leak-scan.sh`) sets it, with `TMPDIR` pointed
//! into its own scan folder, so that it can read every file the tests wrote
//! and then remove the lot. Set it only with `TMPDIR` redirected like that, or
//! the folders pile up in the system's temporary folder.

use std::io;

pub use tempfile::TempDir;

/// The variable that keeps test folders on disk.
pub const KEEP_TEMP_VAR: &str = "ARALO_KEEP_TEMP";

/// A new temporary folder, like `tempfile::tempdir()`. It is deleted on drop
/// unless `ARALO_KEEP_TEMP` is set.
pub fn tempdir() -> io::Result<TempDir> {
    let mut dir = tempfile::tempdir()?;
    if std::env::var_os(KEEP_TEMP_VAR).is_some() {
        dir.disable_cleanup(true);
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_folder_is_made_and_is_empty() {
        let dir = super::tempdir().unwrap();
        assert!(dir.path().is_dir());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
