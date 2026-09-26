import AppKit
import AraloKit
import Sparkle

/// App updates, with Sparkle 2 and an EdDSA-signed appcast on GitHub (plan
/// section 2).
///
/// Sparkle starts only in a build whose Info.plist names a real feed and key,
/// which only the release workflow makes (`UpdateFeed.isConfigured`). A local
/// build never starts it and so contacts nothing. In local-only mode every
/// check is refused, scheduled or asked for (`UpdatePolicy`).
@MainActor
final class Updater: NSObject, SPUUpdaterDelegate {
    private let feed = UpdateFeed(infoDictionary: Bundle.main.infoDictionary ?? [:])
    private let isLocalOnly: () -> Bool
    private var controller: SPUStandardUpdaterController?

    /// `isLocalOnly` is asked before every check. It should answer true when
    /// it cannot tell: a check that should not happen is worse than a missed one.
    init(isLocalOnly: @escaping () -> Bool) {
        self.isLocalOnly = isLocalOnly
        super.init()
        guard feed.isConfigured else { return }
        controller = SPUStandardUpdaterController(
            startingUpdater: true, updaterDelegate: self, userDriverDelegate: nil
        )
    }

    /// Whether Check for Updates belongs in the menu right now.
    var canCheck: Bool {
        controller != nil && UpdatePolicy.mayCheck(feed: feed, localOnly: isLocalOnly())
    }

    func checkForUpdates() {
        guard canCheck else { return }
        controller?.checkForUpdates(nil)
    }

    /// Sparkle asks before every check, the scheduled ones included.
    nonisolated func updater(_ updater: SPUUpdater, mayPerform updateCheck: SPUUpdateCheck) throws {
        let allowed = MainActor.assumeIsolated {
            UpdatePolicy.mayCheck(feed: feed, localOnly: isLocalOnly())
        }
        guard allowed else {
            throw NSError(
                domain: "app.aralo.Aralo.updates", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Update checks are off in local-only mode."]
            )
        }
    }
}
