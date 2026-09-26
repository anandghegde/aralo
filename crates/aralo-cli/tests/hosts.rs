//! No Aralo servers (PRD P7, plan section 9): every host named in what Aralo
//! ships is one `docs/data-flow.md` lists.
//!
//! The page is the allow-list. Its host tables name each host in the first
//! column, in backticks, and say whether Aralo contacts it, and when. This
//! test reads the tables, then looks for URLs in
//!
//! - the `aralo` binary, which holds the whole Rust core the app links, with
//!   every dependency's strings (macOS only, where the app ships: another
//!   platform's TLS and file-system crates carry other text);
//! - the Mac app's Swift sources, tests left out;
//! - the data files compiled into the app or shipped beside it.
//!
//! A URL whose host is not in a table fails the test. Adding one means adding
//! it to the page, which says who sees what goes there.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The hosts `docs/data-flow.md` lists: the first cell of every table row,
/// when it is one host name in backticks.
fn listed_hosts() -> BTreeSet<String> {
    let page = std::fs::read_to_string(repo().join("docs/data-flow.md")).unwrap();
    let hosts: BTreeSet<String> = page
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|rest| rest.split_once('`'))
        .map(|(host, _)| host.to_owned())
        .filter(|host| looks_like_host(host))
        .collect();
    assert!(
        hosts.contains("127.0.0.1") && hosts.contains("api.openai.com"),
        "the host tables in docs/data-flow.md were not found: {hosts:?}"
    );
    hosts
}

fn looks_like_host(text: &str) -> bool {
    !text.is_empty()
        && (text == "localhost" || text.contains('.'))
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
}

/// The host of every `http://` or `https://` URL in `bytes`, lowercased. What
/// follows a scheme and is not a host name (`https://HTTP/0.9` in a
/// dependency's table of versions, say) is not a URL and is skipped.
fn hosts_in(bytes: &[u8]) -> BTreeSet<String> {
    let mut hosts = BTreeSet::new();
    for scheme in [&b"http://"[..], &b"https://"[..]] {
        let mut from = 0;
        while let Some(at) = find(&bytes[from..], scheme) {
            let start = from + at + scheme.len();
            let host: String = bytes[start..]
                .iter()
                .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'.' || **byte == b'-')
                .map(|byte| char::from(*byte).to_ascii_lowercase())
                .collect();
            if looks_like_host(&host) {
                hosts.insert(host);
            }
            from = start;
        }
    }
    hosts
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn unlisted(found: BTreeSet<String>, listed: &BTreeSet<String>) -> Vec<String> {
    found
        .into_iter()
        .filter(|host| !listed.contains(host))
        .collect()
}

fn files_under(folder: &Path, keep: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            // Build output and generated bindings are not sources.
            if matches!(name, ".build" | "Generated" | "DerivedData") {
                continue;
            }
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        } else if keep(&path) {
            found.push(path);
        }
    }
    found.sort();
    found
}

#[cfg(target_os = "macos")]
#[test]
fn the_binary_names_no_host_the_data_flow_page_does_not_list() {
    let binary = std::fs::read(env!("CARGO_BIN_EXE_aralo")).unwrap();
    let found = hosts_in(&binary);
    // The provider presets are in there, so the scan is reading real strings.
    assert!(found.contains("api.openai.com"), "{found:?}");
    let unlisted = unlisted(found, &listed_hosts());
    assert!(
        unlisted.is_empty(),
        "the aralo binary names hosts docs/data-flow.md does not list: {unlisted:?}"
    );
}

#[test]
fn the_mac_apps_sources_name_no_host_the_data_flow_page_does_not_list() {
    let apps = repo().join("apps/macos");
    let sources = files_under(&apps, &|path| {
        path.extension()
            .is_some_and(|extension| extension == "swift")
            && !path.components().any(|part| {
                part.as_os_str()
                    .to_str()
                    .is_some_and(|name| name.ends_with("Tests"))
            })
    });
    assert!(sources.len() > 20, "{} Swift files", sources.len());
    let listed = listed_hosts();
    let mut problems = Vec::new();
    for source in sources {
        let unlisted = unlisted(hosts_in(&std::fs::read(&source).unwrap()), &listed);
        if !unlisted.is_empty() {
            problems.push(format!("{}: {unlisted:?}", source.display()));
        }
    }
    assert!(
        problems.is_empty(),
        "Swift sources name hosts docs/data-flow.md does not list:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_shipped_data_names_no_host_the_data_flow_page_does_not_list() {
    let data = files_under(&repo().join("data"), &|_| true);
    assert!(!data.is_empty());
    let listed = listed_hosts();
    let mut problems = Vec::new();
    for file in data {
        let unlisted = unlisted(hosts_in(&std::fs::read(&file).unwrap()), &listed);
        if !unlisted.is_empty() {
            problems.push(format!("{}: {unlisted:?}", file.display()));
        }
    }
    assert!(
        problems.is_empty(),
        "data files name hosts docs/data-flow.md does not list:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_scan_finds_hosts_and_skips_what_is_not_one() {
    let text = b"see https://Evil.example/x, http://127.0.0.1:11434/v1 and https://HTTP/0.9";
    assert_eq!(
        hosts_in(text),
        BTreeSet::from(["evil.example".to_owned(), "127.0.0.1".to_owned()])
    );
    assert!(!unlisted(hosts_in(text), &listed_hosts()).is_empty());
}
