//! The signed compatibility table download, as the shell sees it (task 5.5).
//!
//! The shell asks for a refresh on the update-check schedule and shows the
//! result; everything else is the core's ([`aralo_core::data_update`]). The
//! request goes through the AI settings' network guard, so local-only mode
//! is enforced there as well as here, and the Swift side opens no connection
//! of its own.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};

use aralo_core::data_update::{self, RefreshError, Refreshed, TableSource};
use aralo_core::signed::DataKey;
use aralo_core::CompatTable;

use crate::{AiProfiles, Core};

/// Where the table in use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CompatTableSource {
    /// Compiled into this build.
    Bundled,
    /// Downloaded from a release and checked against the built-in key.
    Downloaded,
    /// A file named by `load_compat_table`, for trying settings out.
    File,
}

/// What one refresh did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum DataTableRefresh {
    /// A newer signed table is in use now, and saved for the next start.
    Updated { revision: u32 },
    /// The published table is the one in use.
    UpToDate { revision: u32 },
    /// Nothing was requested: this build has no data-table key, or local-only
    /// mode is on.
    NotChecked { reason: String },
    /// A request was made and its answer refused. The table in use stays.
    Failed { message: String },
}

/// The table in use and the last refresh, for a settings line or a report.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CompatTableStatus {
    pub source: CompatTableSource,
    pub revision: u32,
    /// Whether this build can check a download at all. False while
    /// `data/keys/data-tables.pub` is the placeholder.
    pub has_key: bool,
    /// None until a refresh ran in this process.
    pub last_refresh: Option<DataTableRefresh>,
    /// Why a table saved by an earlier refresh was passed over at start.
    pub cache_problem: Option<String>,
}

#[derive(Debug)]
pub(crate) struct TableState {
    source: CompatTableSource,
    last_refresh: Option<DataTableRefresh>,
    cache_problem: Option<String>,
}

/// The table a library starts with: the saved download in `cache`, when it
/// checks against the built-in key and is newer than the bundled table.
pub(crate) fn starting_table(cache: Option<&Path>) -> (CompatTable, TableState) {
    let folder = cache_folder(cache);
    let (table, source, problem) = match &folder {
        Some(folder) => data_update::cached_or_bundled(folder, DataKey::bundled().as_ref()),
        None => (CompatTable::bundled(), TableSource::Bundled, None),
    };
    let state = TableState {
        source: match source {
            TableSource::Bundled => CompatTableSource::Bundled,
            TableSource::Downloaded => CompatTableSource::Downloaded,
        },
        last_refresh: None,
        // No key means nothing was ever saved by this build; not worth saying.
        cache_problem: problem
            .filter(|problem| *problem != RefreshError::NoKey)
            .map(|problem| problem.to_string()),
    };
    (table, state)
}

pub(crate) fn loaded_from_file(state: &mut TableState) {
    state.source = CompatTableSource::File;
}

fn cache_folder(cache: Option<&Path>) -> Option<PathBuf> {
    cache
        .map(Path::to_path_buf)
        .or_else(aralo_core::state::state_folder)
}

impl Core {
    fn table_state(&self) -> std::sync::MutexGuard<'_, TableState> {
        self.shared
            .tables
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[uniffi::export]
impl Core {
    /// The compatibility table in use, and what the last refresh did.
    pub fn compat_table_status(&self) -> CompatTableStatus {
        let state = self.table_state();
        CompatTableStatus {
            source: state.source,
            revision: self.shared.compat().revision(),
            has_key: DataKey::bundled().is_some(),
            last_refresh: state.last_refresh.clone(),
            cache_problem: state.cache_problem.clone(),
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Core {
    /// Fetches the latest release's compatibility table from GitHub through
    /// `ai`'s network guard, and puts it in use when it is signed with the
    /// built-in key and newer than the table in use. Refused before any
    /// request without a key or in local-only mode. A table loaded from a file
    /// stays in use; the download is saved for the next start.
    pub async fn refresh_compat_table(&self, ai: Arc<AiProfiles>) -> DataTableRefresh {
        let outcome = self.refresh_with(&ai).await;
        self.table_state().last_refresh = Some(outcome.clone());
        outcome
    }
}

impl Core {
    async fn refresh_with(&self, ai: &AiProfiles) -> DataTableRefresh {
        let Some(folder) = cache_folder(self.shared.start.cache.as_deref()) else {
            return DataTableRefresh::Failed {
                message: "cannot find Aralo's cache folder".into(),
            };
        };
        let settings = ai.settings();
        let local_only = settings.switches_in_force().local_only;
        let in_use = self
            .shared
            .compat()
            .revision()
            .max(CompatTable::bundled().revision());
        let transport = settings.transport();
        let key = DataKey::bundled();
        match data_update::refresh(&*transport, key.as_ref(), local_only, in_use, &folder).await {
            Ok(Refreshed::Updated(table)) => {
                let revision = table.revision();
                let mut state = self.table_state();
                if state.source != CompatTableSource::File {
                    *self
                        .shared
                        .compat
                        .write()
                        .unwrap_or_else(PoisonError::into_inner) = table;
                    state.source = CompatTableSource::Downloaded;
                    state.cache_problem = None;
                    drop(state);
                    self.engine.refresh_profile();
                }
                DataTableRefresh::Updated { revision }
            }
            Ok(Refreshed::UpToDate { revision }) => DataTableRefresh::UpToDate { revision },
            Err(error @ (RefreshError::NoKey | RefreshError::LocalOnly)) => {
                DataTableRefresh::NotChecked {
                    reason: error.to_string(),
                }
            }
            Err(error) => DataTableRefresh::Failed {
                message: error.to_string(),
            },
        }
    }
}
