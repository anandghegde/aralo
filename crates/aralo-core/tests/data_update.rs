//! Signed table downloads (plan sections 4.4 and 9, task 5.5): the app fetches
//! the latest release's compatibility table and uses it only when it is
//! signed with the built-in key and newer than the table in use.
//!
//! GitHub is played by a fake transport that answers by URL and keeps every
//! request, so a test can say how many were made: none, for a refusal that
//! must happen before the network.

use std::collections::HashMap;
use std::sync::Mutex;

use aralo_ai::guard::NetworkGuard;
use aralo_ai::{AiError, BoxFuture, ByteStream, HttpRequest, HttpResponse, Transport};
use aralo_core::data_update::{
    self, cache_path, cached_or_bundled, load_cached, RefreshError, Refreshed, TableSource,
    MAX_REDIRECTS, MAX_TABLE_BYTES, SIGNATURE_URL, TABLE_URL,
};
use aralo_core::signed::{DataKey, SignedDataError};
use aralo_core::CompatTable;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};

const ASSETS: &str =
    "https://release-assets.githubusercontent.com/github-production-release-asset/1";

#[derive(Clone)]
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn ok(body: impl Into<Vec<u8>>) -> Answer {
    Answer {
        status: 200,
        headers: Vec::new(),
        body: body.into(),
    }
}

fn redirect(to: &str) -> Answer {
    Answer {
        status: 302,
        headers: vec![("location".into(), to.into())],
        body: Vec::new(),
    }
}

/// GitHub, as far as a release download goes.
#[derive(Default)]
struct FakeGitHub {
    answers: Mutex<HashMap<String, Answer>>,
    sent: Mutex<Vec<HttpRequest>>,
}

impl FakeGitHub {
    fn answer(&self, url: &str, answer: Answer) -> &Self {
        self.answers.lock().unwrap().insert(url.into(), answer);
        self
    }

    /// A release whose assets redirect to file storage, as GitHub's do.
    fn publishing(table: &[u8], signature: &str) -> Self {
        let github = Self::default();
        github
            .answer(TABLE_URL, redirect(&format!("{ASSETS}/apps.toml?sig=x")))
            .answer(&format!("{ASSETS}/apps.toml?sig=x"), ok(table))
            .answer(SIGNATURE_URL, redirect(&format!("{ASSETS}/apps.toml.sig")))
            .answer(
                &format!("{ASSETS}/apps.toml.sig"),
                ok(format!("{signature}\n")),
            );
        github
    }

    fn sent(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.url.clone())
            .collect()
    }
}

struct Chunks(Vec<Vec<u8>>);

impl ByteStream for Chunks {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, AiError>> {
        let next = if self.0.is_empty() {
            None
        } else {
            Some(self.0.remove(0))
        };
        Box::pin(async move { Ok(next) })
    }
}

impl Transport for FakeGitHub {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, AiError>> {
        let answer = self.answers.lock().unwrap().get(&request.url).cloned();
        self.sent.lock().unwrap().push(request);
        Box::pin(async move {
            let answer = answer.unwrap_or(Answer {
                status: 404,
                headers: Vec::new(),
                body: Vec::new(),
            });
            // In pieces, as a body arrives.
            let chunks = answer.body.chunks(1000).map(<[u8]>::to_vec).collect();
            Ok(HttpResponse {
                status: answer.status,
                headers: answer.headers,
                body: Box::new(Chunks(chunks)),
            })
        })
    }
}

struct Signer {
    pair: Ed25519KeyPair,
    key: DataKey,
}

impl Signer {
    fn new() -> Self {
        let pair = Ed25519KeyPair::generate().unwrap();
        let public: [u8; 32] = pair.public_key().as_ref().try_into().unwrap();
        Self {
            pair,
            key: DataKey::from_bytes(public),
        }
    }

    fn sign(&self, bytes: &[u8]) -> String {
        self.pair
            .sign(bytes)
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

fn table(revision: u32) -> String {
    format!("version = 0\nrevision = {revision}\n\n[defaults]\ntyping_limit = 40\n")
}

fn bundled_revision() -> u32 {
    CompatTable::bundled().revision()
}

async fn refresh(
    github: &FakeGitHub,
    key: Option<&DataKey>,
    cache: &std::path::Path,
) -> Result<Refreshed, RefreshError> {
    data_update::refresh(github, key, false, bundled_revision(), cache).await
}

#[test]
fn the_shipped_table_has_a_revision() {
    assert!(
        bundled_revision() >= 1,
        "data/compat/apps.toml sets `revision`"
    );
}

#[tokio::test]
async fn a_newer_well_signed_table_is_saved_and_handed_back() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();
    let text = table(bundled_revision() + 1);
    let github = FakeGitHub::publishing(text.as_bytes(), &signer.sign(text.as_bytes()));

    let refreshed = refresh(&github, Some(&signer.key), cache.path())
        .await
        .unwrap();
    let Refreshed::Updated(table) = refreshed else {
        panic!("{refreshed:?}")
    };
    assert_eq!(table.revision(), bundled_revision() + 1);
    assert_eq!(table.defaults().typing_limit, 40);
    assert!(cache_path(cache.path()).exists());
    // The signature first, then the table; both through their redirect.
    assert_eq!(
        github.sent(),
        [
            SIGNATURE_URL.to_owned(),
            format!("{ASSETS}/apps.toml.sig"),
            TABLE_URL.to_owned(),
            format!("{ASSETS}/apps.toml?sig=x"),
        ]
    );
    // What goes to GitHub: a GET with no body and nothing about the user.
    for request in github.sent.lock().unwrap().iter() {
        assert!(request.body.is_none());
        let names: Vec<&str> = request
            .headers
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, ["user-agent", "accept"]);
    }
}

#[tokio::test]
async fn one_changed_byte_is_refused_and_nothing_is_saved() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();
    let text = table(bundled_revision() + 1);
    let signature = signer.sign(text.as_bytes());
    let changed = text.replace("typing_limit = 40", "typing_limit = 41");
    let github = FakeGitHub::publishing(changed.as_bytes(), &signature);

    let error = refresh(&github, Some(&signer.key), cache.path())
        .await
        .unwrap_err();
    assert_eq!(error, RefreshError::Signature(SignedDataError::Mismatch));
    assert!(!cache_path(cache.path()).exists());
}

#[tokio::test]
async fn a_table_signed_by_another_key_is_refused() {
    let cache = aralo_testkit::tempdir().unwrap();
    let ours = Signer::new();
    let theirs = Signer::new();
    let text = table(bundled_revision() + 1);
    let github = FakeGitHub::publishing(text.as_bytes(), &theirs.sign(text.as_bytes()));

    let error = refresh(&github, Some(&ours.key), cache.path())
        .await
        .unwrap_err();
    assert_eq!(error, RefreshError::Signature(SignedDataError::Mismatch));
    assert!(!cache_path(cache.path()).exists());
}

#[tokio::test]
async fn an_older_signed_table_cannot_replace_a_newer_one() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();
    let old = table(3);
    let github = FakeGitHub::publishing(old.as_bytes(), &signer.sign(old.as_bytes()));

    let error = data_update::refresh(&github, Some(&signer.key), false, 5, cache.path())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        RefreshError::Older {
            found: 3,
            in_use: 5
        }
    );
    assert!(!cache_path(cache.path()).exists());

    // The same revision is not news, and is not written again either.
    let same = data_update::refresh(&github, Some(&signer.key), false, 3, cache.path())
        .await
        .unwrap();
    assert_eq!(same, Refreshed::UpToDate { revision: 3 });
    assert!(!cache_path(cache.path()).exists());
}

#[tokio::test]
async fn a_truncated_table_or_a_garbage_signature_is_refused() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();
    let text = table(bundled_revision() + 1);
    let signature = signer.sign(text.as_bytes());

    let truncated = FakeGitHub::publishing(&text.as_bytes()[..text.len() / 2], &signature);
    assert_eq!(
        refresh(&truncated, Some(&signer.key), cache.path())
            .await
            .unwrap_err(),
        RefreshError::Signature(SignedDataError::Mismatch)
    );

    for garbage in ["", "not a signature", &signature[..64], &"zz".repeat(64)] {
        let github = FakeGitHub::publishing(text.as_bytes(), garbage);
        assert_eq!(
            refresh(&github, Some(&signer.key), cache.path())
                .await
                .unwrap_err(),
            RefreshError::Signature(SignedDataError::BadSignatureEncoding),
            "{garbage:?}"
        );
    }
    let github = FakeGitHub::publishing(text.as_bytes(), &signature);
    github.answer(
        &format!("{ASSETS}/apps.toml.sig"),
        ok(vec![0xff, 0xfe, 0x00]),
    );
    assert_eq!(
        refresh(&github, Some(&signer.key), cache.path())
            .await
            .unwrap_err(),
        RefreshError::Signature(SignedDataError::BadSignatureEncoding)
    );
    assert!(!cache_path(cache.path()).exists());
}

#[tokio::test]
async fn well_signed_garbage_is_refused_after_the_signature_check() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();

    let not_text = [0xffu8, 0x00, 0x80, 0x81];
    let github = FakeGitHub::publishing(&not_text, &signer.sign(&not_text));
    assert_eq!(
        refresh(&github, Some(&signer.key), cache.path())
            .await
            .unwrap_err(),
        RefreshError::NotText
    );

    let not_a_table = b"this is not toml [[[";
    let github = FakeGitHub::publishing(not_a_table, &signer.sign(not_a_table));
    assert!(matches!(
        refresh(&github, Some(&signer.key), cache.path())
            .await
            .unwrap_err(),
        RefreshError::Table(_)
    ));

    let too_new = b"version = 99\nrevision = 1000\n";
    let github = FakeGitHub::publishing(too_new, &signer.sign(too_new));
    assert!(matches!(
        refresh(&github, Some(&signer.key), cache.path())
            .await
            .unwrap_err(),
        RefreshError::Table(_)
    ));
    assert!(!cache_path(cache.path()).exists());
}

#[tokio::test]
async fn without_a_key_nothing_is_requested() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();
    let text = table(bundled_revision() + 1);
    let github = FakeGitHub::publishing(text.as_bytes(), &signer.sign(text.as_bytes()));

    assert_eq!(
        refresh(&github, None, cache.path()).await.unwrap_err(),
        RefreshError::NoKey
    );
    assert!(github.sent().is_empty());
}

#[tokio::test]
async fn the_committed_placeholder_key_means_no_download() {
    let key_file = include_str!("../../../data/keys/data-tables.pub");
    if DataKey::from_file_text(key_file).unwrap().is_some() {
        return; // A real key is committed; the other tests cover a set key.
    }
    let cache = aralo_testkit::tempdir().unwrap();
    let github = FakeGitHub::default();
    assert_eq!(DataKey::bundled(), None);
    assert_eq!(
        refresh(&github, DataKey::bundled().as_ref(), cache.path())
            .await
            .unwrap_err(),
        RefreshError::NoKey
    );
    assert!(github.sent().is_empty());
}

#[tokio::test]
async fn in_local_only_mode_nothing_is_requested() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();
    let text = table(bundled_revision() + 1);
    let github = FakeGitHub::publishing(text.as_bytes(), &signer.sign(text.as_bytes()));

    let error = data_update::refresh(&github, Some(&signer.key), true, 0, cache.path())
        .await
        .unwrap_err();
    assert_eq!(error, RefreshError::LocalOnly);
    assert!(github.sent().is_empty());
}

#[tokio::test]
async fn the_network_guard_refuses_github_in_local_only_mode_too() {
    // A caller that forgot to say local-only mode is on still sends nothing:
    // the guard refuses the URL before it opens a connection.
    let cache = aralo_testkit::tempdir().unwrap();
    let guard = NetworkGuard::new(true).unwrap();
    let signer = Signer::new();
    let error = data_update::refresh(&guard, Some(&signer.key), false, 0, cache.path())
        .await
        .unwrap_err();
    assert_eq!(error, RefreshError::LocalOnly);
}

#[tokio::test]
async fn a_redirect_off_the_listed_hosts_is_not_followed() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();
    let github = FakeGitHub::default();
    for (to, expected) in [
        (
            "https://evil.example/apps.toml.sig",
            RefreshError::Host("evil.example".into()),
        ),
        (
            "https://127.0.0.1/apps.toml.sig",
            RefreshError::Host("127.0.0.1".into()),
        ),
        (
            "https://github.com:8443/apps.toml.sig",
            RefreshError::Host("github.com".into()),
        ),
        (
            "http://release-assets.githubusercontent.com/apps.toml.sig",
            RefreshError::Redirect("it is not an https address".into()),
        ),
        (
            "/relative/apps.toml.sig",
            RefreshError::Redirect("it is not an https address".into()),
        ),
    ] {
        github.sent.lock().unwrap().clear();
        github.answer(SIGNATURE_URL, redirect(to));
        assert_eq!(
            refresh(&github, Some(&signer.key), cache.path())
                .await
                .unwrap_err(),
            expected,
            "{to}"
        );
        assert_eq!(github.sent(), [SIGNATURE_URL], "{to}");
    }
}

#[tokio::test]
async fn a_redirect_loop_stops() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();
    let github = FakeGitHub::default();
    github.answer(SIGNATURE_URL, redirect(SIGNATURE_URL));
    assert!(matches!(
        refresh(&github, Some(&signer.key), cache.path())
            .await
            .unwrap_err(),
        RefreshError::Redirect(_)
    ));
    assert_eq!(github.sent().len(), MAX_REDIRECTS + 1);
}

#[tokio::test]
async fn an_error_status_or_an_oversized_table_is_refused() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();

    let missing = FakeGitHub::default();
    assert_eq!(
        refresh(&missing, Some(&signer.key), cache.path())
            .await
            .unwrap_err(),
        RefreshError::Status(404)
    );

    let huge = vec![b'#'; MAX_TABLE_BYTES + 1];
    let github = FakeGitHub::publishing(&huge, &signer.sign(&huge));
    assert_eq!(
        refresh(&github, Some(&signer.key), cache.path())
            .await
            .unwrap_err(),
        RefreshError::TooLarge(MAX_TABLE_BYTES)
    );
    assert!(!cache_path(cache.path()).exists());
}

#[tokio::test]
async fn the_saved_table_is_what_the_next_start_uses() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();
    let text = table(bundled_revision() + 1);
    let github = FakeGitHub::publishing(text.as_bytes(), &signer.sign(text.as_bytes()));
    refresh(&github, Some(&signer.key), cache.path())
        .await
        .unwrap();

    let (table, source, problem) = cached_or_bundled(cache.path(), Some(&signer.key));
    assert_eq!(source, TableSource::Downloaded);
    assert_eq!(problem, None);
    assert_eq!(table, CompatTable::parse(&text).unwrap());

    // Checked again at start: another key, or none, and it is passed over.
    let (table, source, problem) = cached_or_bundled(cache.path(), Some(&Signer::new().key));
    assert_eq!(
        (source, table),
        (TableSource::Bundled, CompatTable::bundled())
    );
    assert_eq!(
        problem,
        Some(RefreshError::Signature(SignedDataError::Mismatch))
    );
    let (_, source, problem) = cached_or_bundled(cache.path(), None);
    assert_eq!(source, TableSource::Bundled);
    assert_eq!(problem, Some(RefreshError::NoKey));
}

#[test]
fn a_cache_that_was_edited_or_torn_is_passed_over() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();
    let text = table(bundled_revision() + 1);
    let path = cache_path(cache.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();

    let edited = text.replace("40", "41");
    std::fs::write(&path, format!("{}\n{edited}", signer.sign(text.as_bytes()))).unwrap();
    let (table, source, problem) = cached_or_bundled(cache.path(), Some(&signer.key));
    assert_eq!(
        (source, table),
        (TableSource::Bundled, CompatTable::bundled())
    );
    assert_eq!(
        problem,
        Some(RefreshError::Signature(SignedDataError::Mismatch))
    );

    std::fs::write(&path, &signer.sign(text.as_bytes())[..40]).unwrap();
    let (_, source, problem) = cached_or_bundled(cache.path(), Some(&signer.key));
    assert_eq!(source, TableSource::Bundled);
    assert!(problem.is_some());
}

#[test]
fn a_build_with_a_newer_table_than_the_cache_uses_its_own() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();
    let path = cache_path(cache.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text = table(bundled_revision());
    std::fs::write(&path, format!("{}\n{text}", signer.sign(text.as_bytes()))).unwrap();

    let (table, source, problem) = cached_or_bundled(cache.path(), Some(&signer.key));
    assert_eq!(
        (source, table),
        (TableSource::Bundled, CompatTable::bundled())
    );
    assert_eq!(problem, None);
    assert_eq!(
        load_cached(cache.path(), Some(&signer.key), bundled_revision() - 1)
            .unwrap()
            .map(|table| table.revision()),
        Some(bundled_revision())
    );
}

#[test]
fn no_cache_is_the_bundled_table_and_no_problem() {
    let cache = aralo_testkit::tempdir().unwrap();
    let (table, source, problem) = cached_or_bundled(cache.path(), None);
    assert_eq!(
        (source, table, problem),
        (TableSource::Bundled, CompatTable::bundled(), None)
    );
}

#[tokio::test]
async fn a_second_download_replaces_the_first() {
    let cache = aralo_testkit::tempdir().unwrap();
    let signer = Signer::new();
    for revision in [bundled_revision() + 1, bundled_revision() + 2] {
        let text = table(revision);
        let github = FakeGitHub::publishing(text.as_bytes(), &signer.sign(text.as_bytes()));
        data_update::refresh(
            &github,
            Some(&signer.key),
            false,
            revision - 1,
            cache.path(),
        )
        .await
        .unwrap();
    }
    let (table, _, _) = cached_or_bundled(cache.path(), Some(&signer.key));
    assert_eq!(table.revision(), bundled_revision() + 2);
    let names: Vec<_> = std::fs::read_dir(cache_path(cache.path()).parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        names,
        [data_update::CACHE_FILE],
        "no temporary file is left"
    );
}
