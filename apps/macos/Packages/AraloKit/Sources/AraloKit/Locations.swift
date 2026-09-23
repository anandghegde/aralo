import Foundation

/// Where Aralo keeps things, and what an environment variable may say about it.
///
/// Separate from the service that opens them: these are answers about the disk,
/// the same before the library is open as after, and the harness and the tests
/// set them from the environment rather than from a window.
public extension AraloService {
    /// Where the library lives unless the user chose somewhere else: a visible
    /// folder that is easy to sync, outside the folders macOS guards with
    /// their own permission prompts.
    static var defaultLibraryURL: URL {
        if let override = ProcessInfo.processInfo.environment["ARALO_LIBRARY"], !override.isEmpty {
            return URL(fileURLWithPath: override, isDirectory: true)
        }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Aralo", isDirectory: true)
    }

    /// Where the search index and the rest of what Aralo can rebuild goes.
    /// Nothing in here is the user's work: deleting it costs a rebuild.
    static var defaultCacheURL: URL {
        if let override = ProcessInfo.processInfo.environment["ARALO_STATE"], !override.isEmpty {
            return URL(fileURLWithPath: override, isDirectory: true)
        }
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent("Library/Application Support", isDirectory: true)
        return support.appendingPathComponent("Aralo", isDirectory: true)
    }

    /// A compatibility table to use instead of the one built into the core,
    /// for measuring an app without rebuilding. Nothing a user sets.
    static var compatTableOverride: String? {
        let path = ProcessInfo.processInfo.environment["ARALO_COMPAT"] ?? ""
        return path.isEmpty ? nil : path
    }
}
