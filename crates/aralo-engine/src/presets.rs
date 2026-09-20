//! Apps Aralo stays out of by default (PRD P2).
//!
//! Secure-input detection in the shell already covers password fields. This
//! list covers apps where every field is sensitive. It lives in code, not in a
//! data file, so it is part of what a reviewer of this crate reads. Additions
//! arrive through "app compatibility report" issues; users add their own apps
//! in settings.

/// Bundle IDs, compared without regard to ASCII case.
pub const EXCLUDED_APP_PRESETS: &[&str] = &[
    "com.1password.1password",
    "com.agilebits.onepassword7",
    "com.apple.keychainaccess",
    "com.apple.Passwords",
    "com.bitwarden.desktop",
    "org.keepassxc.keepassxc",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_well_formed_bundle_ids() {
        for app in EXCLUDED_APP_PRESETS {
            assert!(app.contains('.'), "{app} is not a reverse-DNS bundle ID");
            assert!(!app.contains(char::is_whitespace));
        }
        let mut sorted: alloc::vec::Vec<_> = EXCLUDED_APP_PRESETS
            .iter()
            .map(|app| app.to_ascii_lowercase())
            .collect();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), EXCLUDED_APP_PRESETS.len(), "duplicate preset");
    }
}
