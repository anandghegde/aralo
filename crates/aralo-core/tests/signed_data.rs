//! Signed data updates (plan section 9): a downloaded compatibility table is
//! used only when its Ed25519 signature checks against the key compiled into
//! the build. A bad signature keeps the bundled copy.
//!
//! `fixtures/signed/` holds a table signed by `scripts/sign-data.sh`, the
//! tool the release workflow signs with, and the public key it was signed
//! with. The core must accept exactly that format.

use std::path::{Path, PathBuf};

use aralo_core::signed::{DataKey, SignedDataError};
use aralo_core::CompatTable;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/signed")
        .join(name)
}

fn read(name: &str) -> String {
    std::fs::read_to_string(fixture(name)).unwrap()
}

fn release_key() -> DataKey {
    DataKey::from_hex(&read("fixture-key.hex")).unwrap()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A key pair made for one test, standing in for somebody else's key.
fn other_signer() -> (Ed25519KeyPair, DataKey) {
    let pair = Ed25519KeyPair::generate().unwrap();
    let public: [u8; 32] = pair.public_key().as_ref().try_into().unwrap();
    (pair, DataKey::from_bytes(public))
}

#[test]
fn a_table_signed_by_the_release_script_is_accepted() {
    let text = read("apps.toml");
    let table = CompatTable::from_signed(&text, &read("apps.toml.sig"), Some(&release_key()))
        .expect("the release script's signature must check");
    assert_eq!(table, CompatTable::parse(&text).unwrap());
}

#[test]
fn one_changed_byte_is_refused_and_the_bundled_copy_stays() {
    let mut text = read("apps.toml");
    text = text.replacen("typing_limit = 120", "typing_limit = 121", 1);
    assert_ne!(
        text,
        read("apps.toml"),
        "the fixture must hold the line this test edits"
    );

    let mut in_use = CompatTable::bundled();
    let error = in_use
        .update_from_signed(&text, &read("apps.toml.sig"), Some(&release_key()))
        .unwrap_err();
    assert_eq!(error, SignedDataError::Mismatch);
    assert_eq!(in_use, CompatTable::bundled());
}

#[test]
fn a_signature_by_another_key_is_refused() {
    let text = read("apps.toml");
    let (pair, _) = other_signer();
    let signature = hex(pair.sign(text.as_bytes()).as_ref());

    let mut in_use = CompatTable::bundled();
    let error = in_use
        .update_from_signed(&text, &signature, Some(&release_key()))
        .unwrap_err();
    assert_eq!(error, SignedDataError::Mismatch);
    assert_eq!(in_use, CompatTable::bundled());
}

#[test]
fn without_a_key_nothing_is_accepted() {
    let mut in_use = CompatTable::bundled();
    let error = in_use
        .update_from_signed(&read("apps.toml"), &read("apps.toml.sig"), None)
        .unwrap_err();
    assert_eq!(error, SignedDataError::NoKey);
    assert_eq!(in_use, CompatTable::bundled());
}

#[test]
fn a_signature_that_is_not_hex_is_refused() {
    let key = release_key();
    for signature in ["", "00", &"zz".repeat(64), &"00".repeat(65)] {
        assert_eq!(
            CompatTable::from_signed(&read("apps.toml"), signature, Some(&key)).unwrap_err(),
            SignedDataError::BadSignatureEncoding,
            "{signature:?}"
        );
    }
}

#[test]
fn a_well_signed_table_that_does_not_parse_is_refused_too() {
    let (pair, key) = other_signer();
    let text = "version = 99\n";
    let signature = hex(pair.sign(text.as_bytes()).as_ref());

    let mut in_use = CompatTable::bundled();
    let error = in_use
        .update_from_signed(text, &signature, Some(&key))
        .unwrap_err();
    assert!(matches!(error, SignedDataError::Table(_)), "{error:?}");
    assert_eq!(in_use, CompatTable::bundled());
}

#[test]
fn a_well_signed_table_replaces_the_one_in_use() {
    let (pair, key) = other_signer();
    let text = "version = 0\n\n[defaults]\ntyping_limit = 40\n";
    let signature = hex(pair.sign(text.as_bytes()).as_ref());

    let mut in_use = CompatTable::bundled();
    in_use
        .update_from_signed(text, &signature, Some(&key))
        .unwrap();
    assert_eq!(in_use.defaults().typing_limit, 40);
}

#[test]
fn the_key_file_reads_unset_as_no_key() {
    let text = "# a comment\n\nunset\n";
    assert_eq!(DataKey::from_file_text(text).unwrap(), None);
    let line = read("fixture-key.hex");
    assert_eq!(
        DataKey::from_file_text(&format!("# key\n{line}")).unwrap(),
        Some(release_key())
    );
    assert_eq!(
        DataKey::from_file_text("not a key").unwrap_err(),
        SignedDataError::BadKey
    );
}
