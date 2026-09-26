//! Signed table downloads (plan sections 4.4 and 9, task 5.5).
//!
//! The compatibility table ships inside the build and a newer one is
//! published with every release, as the assets `apps.toml` and
//! `apps.toml.sig` (`scripts/release.sh`). [`refresh`] fetches the latest
//! release's pair, and a table is used only when all of this holds, checked
//! in this order:
//!
//! 1. The build has a data-table key. While `data/keys/data-tables.pub` says
//!    `unset`, [`refresh`] refuses before any request is made.
//! 2. Local-only mode is off. It is checked here, before any request, and the
//!    network guard the request goes through checks it again.
//! 3. Every URL, the first and every redirect, is https on port 443 to one of
//!    [`DOWNLOAD_HOSTS`], the hosts `docs/data-flow.md` lists for GitHub
//!    releases. The guard follows no redirect itself, so this module follows
//!    them, at most [`MAX_REDIRECTS`] times.
//! 4. Neither answer is larger than its limit.
//! 5. The Ed25519 signature checks against the built-in key over the bytes as
//!    they arrived. Nothing is parsed or written before this.
//! 6. The bytes are UTF-8 and parse as a table this build reads.
//! 7. Its [`CompatTable::revision`] is higher than the table in use. The
//!    revision is inside the signed bytes, so a replayed older table, signed
//!    as it is, is refused: no rollback.
//!
//! Only then is the pair written to the cache, as one file replaced
//! atomically, and handed back to be put in use. At start
//! [`cached_or_bundled`] checks the cached file the same way, steps 5 to 7
//! against the bundled table, so a cache that was tampered with, torn or
//! outlived by a newer build is ignored and the bundled table used.
//!
//! Every failure leaves the table in use as it was.

use std::path::{Path, PathBuf};
use std::time::Duration;

use aralo_ai::{AiError, ByteStream, HttpRequest, Method, Refusal, Transport};

use crate::compat::{CompatError, CompatTable};
use crate::signed::{DataKey, SignedDataError};

/// The latest release's table, from the repository `scripts/release.sh`
/// publishes to. GitHub answers with a redirect to its file
/// storage.
pub const TABLE_URL: &str =
    "https://github.com/anandghegde/aralo/releases/latest/download/apps.toml";
/// Its signature: 128 hex digits.
pub const SIGNATURE_URL: &str =
    "https://github.com/anandghegde/aralo/releases/latest/download/apps.toml.sig";
/// Every host a download may be sent to, redirects included.
pub const DOWNLOAD_HOSTS: [&str; 3] = [
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];
/// The most redirects one download follows.
pub const MAX_REDIRECTS: usize = 5;
/// The largest table accepted. The one that ships is a few kilobytes.
pub const MAX_TABLE_BYTES: usize = 256 * 1024;
/// The largest signature file accepted: 128 hex digits and a line break.
pub const MAX_SIGNATURE_BYTES: usize = 1024;
/// How long one refresh may take, both downloads together.
pub const REFRESH_TIMEOUT: Duration = Duration::from_secs(60);
/// The folder in the cache folder the downloaded table is kept in.
pub const CACHE_FOLDER: &str = "data-tables";
/// The cached table: its signature on the first line, then the table's bytes
/// exactly as they were signed.
pub const CACHE_FILE: &str = "apps.toml.signed";
/// What the requests say they come from. Nothing about the user, the Mac or
/// the version.
const USER_AGENT: &str = "Aralo";

/// Why a refresh changed nothing. The table in use stays in use.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefreshError {
    #[error("this build has no data-table key, so no table is downloaded")]
    NoKey,
    #[error("local-only mode is on, so no table is downloaded")]
    LocalOnly,
    #[error("the download failed: {0}")]
    Network(String),
    #[error("the download took longer than {} seconds", REFRESH_TIMEOUT.as_secs())]
    TimedOut,
    #[error("GitHub answered {0}")]
    Status(u16),
    #[error("refused to download from {0}")]
    Host(String),
    #[error("a redirect could not be followed: {0}")]
    Redirect(String),
    #[error("the download is larger than {0} bytes")]
    TooLarge(usize),
    #[error(transparent)]
    Signature(SignedDataError),
    #[error("the table is not UTF-8 text")]
    NotText,
    #[error(transparent)]
    Table(#[from] CompatError),
    #[error("the table is revision {found}, older than revision {in_use} in use")]
    Older { found: u32, in_use: u32 },
    #[error("the downloaded table could not be saved: {0}")]
    Cache(String),
}

impl From<SignedDataError> for RefreshError {
    fn from(error: SignedDataError) -> Self {
        match error {
            SignedDataError::NoKey => Self::NoKey,
            SignedDataError::Table(error) => Self::Table(error),
            other => Self::Signature(other),
        }
    }
}

/// What a refresh that succeeded found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refreshed {
    /// A newer table, checked and saved to the cache. The caller puts it in use.
    Updated(CompatTable),
    /// The published table is the revision in use.
    UpToDate { revision: u32 },
}

/// Where the table in use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableSource {
    /// Compiled into this build.
    Bundled,
    /// Downloaded, and read back from the cache.
    Downloaded,
}

/// The cached table's path in `cache`.
pub fn cache_path(cache: &Path) -> PathBuf {
    cache.join(CACHE_FOLDER).join(CACHE_FILE)
}

/// Fetches the latest published table through `transport`, which in the app
/// is the network guard, and saves it to `cache` if it is signed with `key`
/// and newer than revision `in_use`. See the module's documentation for every
/// check, and the order they run in.
pub async fn refresh(
    transport: &dyn Transport,
    key: Option<&DataKey>,
    local_only: bool,
    in_use: u32,
    cache: &Path,
) -> Result<Refreshed, RefreshError> {
    let key = key.ok_or(RefreshError::NoKey)?;
    if local_only {
        return Err(RefreshError::LocalOnly);
    }
    let downloads = async {
        let signature = get(transport, SIGNATURE_URL, MAX_SIGNATURE_BYTES).await?;
        let table = get(transport, TABLE_URL, MAX_TABLE_BYTES).await?;
        Ok::<_, RefreshError>((signature, table))
    };
    let (signature, table) = tokio::time::timeout(REFRESH_TIMEOUT, downloads)
        .await
        .map_err(|_| RefreshError::TimedOut)??;
    let signature = std::str::from_utf8(&signature)
        .map_err(|_| RefreshError::Signature(SignedDataError::BadSignatureEncoding))?
        .trim()
        .to_owned();
    let parsed = check(&table, &signature, key)?;
    if parsed.revision() < in_use {
        return Err(RefreshError::Older {
            found: parsed.revision(),
            in_use,
        });
    }
    if parsed.revision() == in_use {
        return Ok(Refreshed::UpToDate { revision: in_use });
    }
    save(cache, &signature, &table)?;
    Ok(Refreshed::Updated(parsed))
}

/// The table to start with: the cached download when it checks against `key`
/// and is newer than the bundled table, and the bundled table otherwise. The
/// error says why a cached table that exists was passed over.
pub fn cached_or_bundled(
    cache: &Path,
    key: Option<&DataKey>,
) -> (CompatTable, TableSource, Option<RefreshError>) {
    let bundled = CompatTable::bundled();
    match load_cached(cache, key, bundled.revision()) {
        Ok(Some(table)) => (table, TableSource::Downloaded, None),
        Ok(None) => (bundled, TableSource::Bundled, None),
        Err(error) => (bundled, TableSource::Bundled, Some(error)),
    }
}

/// The cached table, if there is one, it checks against `key`, and its
/// revision is above `floor`. `Ok(None)` when there is none, or it is not
/// newer: a build that ships a newer table than it once downloaded uses its
/// own.
pub fn load_cached(
    cache: &Path,
    key: Option<&DataKey>,
    floor: u32,
) -> Result<Option<CompatTable>, RefreshError> {
    let path = cache_path(cache);
    let bytes = match read_limited_file(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(RefreshError::Cache(error.to_string())),
    };
    let key = key.ok_or(RefreshError::NoKey)?;
    let split = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or(RefreshError::Signature(
            SignedDataError::BadSignatureEncoding,
        ))?;
    let signature = std::str::from_utf8(&bytes[..split])
        .map_err(|_| RefreshError::Signature(SignedDataError::BadSignatureEncoding))?;
    let table = check(&bytes[split + 1..], signature, key)?;
    Ok((table.revision() > floor).then_some(table))
}

/// Steps 5 and 6: the signature over the bytes as they are, then the text.
fn check(bytes: &[u8], signature: &str, key: &DataKey) -> Result<CompatTable, RefreshError> {
    key.verify(bytes, signature)?;
    let text = std::str::from_utf8(bytes).map_err(|_| RefreshError::NotText)?;
    Ok(CompatTable::parse(text)?)
}

fn save(cache: &Path, signature: &str, table: &[u8]) -> Result<(), RefreshError> {
    let path = cache_path(cache);
    let failed = |error: std::io::Error| RefreshError::Cache(error.to_string());
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(failed)?;
    }
    let mut contents = Vec::with_capacity(signature.len() + 1 + table.len());
    contents.extend_from_slice(signature.as_bytes());
    contents.push(b'\n');
    contents.extend_from_slice(table);
    aralo_library::write_atomic(&path, &contents).map_err(failed)
}

fn read_limited_file(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let limit = MAX_TABLE_BYTES + MAX_SIGNATURE_BYTES;
    let mut bytes = Vec::new();
    file.take(u64::try_from(limit + 1).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the cached table is too large",
        ));
    }
    Ok(bytes)
}

/// Step 3: https, port 443, and one of the listed hosts.
fn admit(url: &str) -> Result<(), RefreshError> {
    let endpoint = aralo_ai::guard::parse_endpoint(url)
        .map_err(|error| RefreshError::Host(error.to_string()))?;
    let allowed = match &endpoint.host {
        aralo_ai::Host::Name(name) => DOWNLOAD_HOSTS.contains(&name.as_str()),
        aralo_ai::Host::Ip(_) => false,
    };
    if endpoint.https && endpoint.port == 443 && allowed {
        Ok(())
    } else {
        Err(RefreshError::Host(match &endpoint.host {
            aralo_ai::Host::Name(name) => name.clone(),
            aralo_ai::Host::Ip(ip) => ip.to_string(),
        }))
    }
}

/// One download, following at most [`MAX_REDIRECTS`] redirects, each to a
/// listed host.
async fn get(transport: &dyn Transport, url: &str, limit: usize) -> Result<Vec<u8>, RefreshError> {
    let mut url = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        admit(&url)?;
        let response = transport
            .send(HttpRequest {
                method: Method::Get,
                url: url.clone(),
                headers: vec![
                    ("user-agent".into(), USER_AGENT.into()),
                    ("accept".into(), "application/octet-stream".into()),
                ],
                body: None,
            })
            .await
            .map_err(network_error)?;
        match response.status {
            200 => return read_limited(response.body, limit).await,
            301 | 302 | 303 | 307 | 308 => {
                let location = response
                    .header("location")
                    .ok_or_else(|| RefreshError::Redirect("it names no location".into()))?;
                // GitHub's are absolute. A relative one is refused rather
                // than resolved.
                if !location.starts_with("https://") {
                    return Err(RefreshError::Redirect("it is not an https address".into()));
                }
                url = location.to_owned();
            }
            status => return Err(RefreshError::Status(status)),
        }
    }
    Err(RefreshError::Redirect(format!(
        "more than {MAX_REDIRECTS} redirects"
    )))
}

async fn read_limited(
    mut body: Box<dyn ByteStream>,
    limit: usize,
) -> Result<Vec<u8>, RefreshError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next_chunk().await.map_err(network_error)? {
        if bytes.len() + chunk.len() > limit {
            return Err(RefreshError::TooLarge(limit));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn network_error(error: AiError) -> RefreshError {
    match error {
        AiError::Refused(Refusal::LocalOnly { .. }) => RefreshError::LocalOnly,
        AiError::Refused(refusal) => RefreshError::Network(refusal.to_string()),
        other => RefreshError::Network(other.to_string()),
    }
}
