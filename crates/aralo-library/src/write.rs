use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

/// Writes a file so that a reader, a sync client or a crash never sees half of
/// it: write a temporary file in the same folder, flush it to disk, then rename
/// it over the target. The temporary name starts with a dot, so the loader and
/// most sync clients ignore it.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let folder = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".{}.aralo-tmp", std::process::id()));
    let temporary = folder.join(temporary_name);

    let result = (|| {
        let mut file = File::create(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_the_target_and_leaves_no_temporary_file() {
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join("note.md");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        let names: Vec<_> = fs::read_dir(folder.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["note.md"]);
    }

    #[test]
    fn a_failed_write_leaves_the_old_contents() {
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join("missing-folder").join("note.md");
        assert!(write_atomic(&path, b"x").is_err());
        assert!(!path.exists());
    }
}
