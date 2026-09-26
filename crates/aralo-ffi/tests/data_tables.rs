//! The signed compatibility table download through the bridge (task 5.5).
//! The checks themselves are tested in `aralo-core/tests/data_update.rs`;
//! these cover what the shell sees.

use aralo_ffi::{AiProfiles, CompatTableSource, Core, DataTableRefresh, KeyStorage};

fn placeholder_key() -> bool {
    include_str!("../../../data/keys/data-tables.pub")
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        == Some("unset")
}

fn open(folder: &std::path::Path) -> std::sync::Arc<Core> {
    Core::open_library(
        folder.join("Aralo").to_string_lossy().into_owned(),
        Some(folder.join("cache").to_string_lossy().into_owned()),
        None,
        None,
    )
    .unwrap()
}

#[test]
fn a_new_library_uses_the_bundled_table() {
    let folder = aralo_testkit::tempdir().unwrap();
    let status = open(folder.path()).compat_table_status();
    assert_eq!(status.source, CompatTableSource::Bundled);
    assert!(status.revision >= 1);
    assert_eq!(status.last_refresh, None);
    assert_eq!(status.cache_problem, None);
    assert_eq!(status.has_key, !placeholder_key());
}

#[tokio::test]
async fn with_the_placeholder_key_a_refresh_asks_nothing_and_changes_nothing() {
    if !placeholder_key() {
        return;
    }
    let folder = aralo_testkit::tempdir().unwrap();
    let core = open(folder.path());
    let ai = AiProfiles::open(
        Some(folder.path().join("profiles.toml").display().to_string()),
        KeyStorage::Memory,
    )
    .unwrap();
    let before = core.compat_table_status();

    let outcome = core.refresh_compat_table(ai).await;
    assert!(
        matches!(&outcome, DataTableRefresh::NotChecked { reason } if reason.contains("no data-table key")),
        "{outcome:?}"
    );
    let after = core.compat_table_status();
    assert_eq!(after.source, CompatTableSource::Bundled);
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.last_refresh, Some(outcome));
    assert!(!folder.path().join("cache/data-tables").exists());
}

#[test]
fn a_saved_table_this_build_cannot_check_is_passed_over() {
    if !placeholder_key() {
        return;
    }
    let folder = aralo_testkit::tempdir().unwrap();
    let saved = folder.path().join("cache/data-tables/apps.toml.signed");
    std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
    std::fs::write(
        &saved,
        format!("{}\nversion = 0\nrevision = 999\n", "00".repeat(64)),
    )
    .unwrap();
    let status = open(folder.path()).compat_table_status();
    assert_eq!(status.source, CompatTableSource::Bundled);
    assert!(status.revision < 999);
}

#[test]
fn a_table_loaded_from_a_file_says_so() {
    let folder = aralo_testkit::tempdir().unwrap();
    let core = open(folder.path());
    let path = folder.path().join("try.toml");
    std::fs::write(&path, "version = 0\nrevision = 0\n").unwrap();
    core.load_compat_table(path.display().to_string()).unwrap();
    let status = core.compat_table_status();
    assert_eq!(status.source, CompatTableSource::File);
    assert_eq!(status.revision, 0);
}
