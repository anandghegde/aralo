//! The starter library, embedded so a new install expands something before the
//! user has written a single snippet, with no network involved.

use std::io;
use std::path::Path;

use aralo_library::write_atomic;

macro_rules! starter_files {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_str!(concat!("../../../data/starter/", $path)))),*]
    };
}

/// Every file under `data/starter`, by its path from the library root. A test
/// compares this list with the folder, so a new starter file cannot be missed.
pub const FILES: &[(&str, &str)] = starter_files![
    "README.md",
    "Basics/_group.yaml",
    "Basics/best-regards.md",
    "Basics/on-my-way.md",
    "Basics/shrug.md",
    "Basics/thanks.md",
    "Symbols/_group.yaml",
    "Symbols/arrow-left.md",
    "Symbols/arrow-right.md",
    "Symbols/check.md",
    "Symbols/ellipsis.md",
    "Symbols/not-equal.md",
];

/// Writes the starter files that are not already there. Returns how many it
/// wrote. A file the user already has is never replaced.
pub(crate) fn install(root: &Path) -> io::Result<usize> {
    let mut written = 0;
    for (relative, contents) in FILES {
        let path = root.join(relative);
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(&path, contents.as_bytes())?;
        written += 1;
    }
    Ok(written)
}
