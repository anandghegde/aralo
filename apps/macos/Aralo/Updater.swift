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
///
/// The signed data tables follow the same schedule (plan section 4.4): every
/// check Sparkle is allowed to make also asks the core to refresh the
/// compatibility table, through `onCheck`. So they are never fetched in
/// local-only mode, never by a build Sparkle does not run in, and not
/// automatically when the user has turned automatic update checks off. The
/// fetch itself is the core's, through its network guard.
@MainActor
final class Updater: NSObject, SPUUpdaterDelegate {
    private let feed = UpdateFeed(infoDictionary: Bundle.main.infoDictionary ?? [:])
    private let isLocalOnly: () -> Bool
    private let onCheck: () -> Void
    private var controller: SPUStandardUpdaterController?

    /// `isLocalOnly` is asked before every check. It should answer true when
    /// it cannot tell: a check that should not happen is worse than a missed one.
    /// `onCheck` runs on the main thread after a check has been allowed.
    init(isLocalOnly: @escaping () -> Bool, onCheck: @escaping () -> Void = {}) {
        self.isLocalOnly = isLocalOnly
        self.onCheck = onCheck
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
            let allowed = UpdatePolicy.mayCheck(feed: feed, localOnly: isLocalOnly())
            if allowed { onCheck() }
            return allowed
        }
        guard allowed else {
            throw NSError(
                domain: "app.aralo.Aralo.updates", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Update checks are off in local-only mode."]
            )
        }
    }
}
