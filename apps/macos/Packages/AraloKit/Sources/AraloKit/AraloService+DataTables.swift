import AraloBridge
import Foundation

extension AraloService {
    /// The compatibility table in use: bundled, downloaded or from a file,
    /// and what the last refresh did.
    public var compatTableStatus: CompatTableStatus? { core?.compatTableStatus() }

    /// Asks the core for the latest signed compatibility table (task 5.5).
    /// The app calls this on the update-check schedule (`Updater`). The core
    /// fetches it through the AI settings' network guard and refuses before
    /// any request without a data-table key or in local-only mode; this opens
    /// no connection of its own.
    public func refreshCompatTable() {
        guard let core, let settings = try? aiSettings() else { return }
        Task { _ = await core.refreshCompatTable(ai: settings) }
    }
}
