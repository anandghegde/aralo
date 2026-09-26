import Foundation

/// Where Sparkle looks for updates and the key it checks them with, as the
/// app's Info.plist gives them (`SUFeedURL`, `SUPublicEDKey`).
///
/// A local build carries the placeholders from `apps/macos/project.yml`, and
/// is not configured: Sparkle is never started in it, so it contacts nothing.
/// The release workflow fills both in (`scripts/release.sh`).
public struct UpdateFeed: Equatable, Sendable {
    public let url: URL?
    public let publicKey: String?

    public init(url: URL?, publicKey: String?) {
        self.url = url
        self.publicKey = publicKey
    }

    public init(infoDictionary: [String: Any]) {
        let address = (infoDictionary["SUFeedURL"] as? String)?.trimmingCharacters(in: .whitespaces)
        self.init(
            url: address.flatMap { $0.isEmpty ? nil : URL(string: $0) },
            publicKey: (infoDictionary["SUPublicEDKey"] as? String)?.trimmingCharacters(in: .whitespaces)
        )
    }

    /// An https feed on GitHub, the one host data-flow.md lists for updates,
    /// and an Ed25519 public key: 32 bytes in base64. A placeholder is
    /// neither, so it never passes.
    public var isConfigured: Bool {
        guard let url, url.scheme == "https", url.host == "github.com" else { return false }
        guard let publicKey, let key = Data(base64Encoded: publicKey) else { return false }
        return key.count == 32
    }
}

/// Whether the app may ask the feed for an update.
///
/// Sparkle's check is the one request Aralo makes on its own behalf (plan
/// section 9, data-flow.md). Local-only mode means no network, so it turns the
/// check off too, whatever Sparkle's own settings say.
public enum UpdatePolicy {
    public static func mayCheck(feed: UpdateFeed, localOnly: Bool) -> Bool {
        feed.isConfigured && !localOnly
    }
}
