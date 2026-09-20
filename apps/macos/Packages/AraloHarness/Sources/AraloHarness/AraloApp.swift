import AppKit

/// Starts and stops the Aralo under test.
@MainActor
public enum AraloApp {
    public static let bundleID = "app.aralo.Aralo"

    public enum Problem: Error, CustomStringConvertible {
        case notBuilt(String)
        case didNotStart(String)
        case noTap

        public var description: String {
            switch self {
            case .notBuilt(let path): "there is no app at \(path); run `make app`"
            case .didNotStart(let why): "Aralo did not start: \(why)"
            case .noTap: """
                Aralo is running, but macOS has not let it watch the keyboard, so nothing would expand. \
                Allow that build under System Settings › Privacy & Security › Accessibility and Input Monitoring. \
                An ad hoc signed build loses the grant with every rebuild: sign the build under test with a \
                fixed identity (`make app SIGN_IDENTITY=...`).
                """
            }
        }
    }

    public static var isRunning: Bool {
        !NSRunningApplication.runningApplications(withBundleIdentifier: bundleID).isEmpty
    }

    /// Whether a running Aralo has an event tap that is switched on, which is
    /// the proof that macOS lets it read keys. Without this check a missing
    /// grant would read as fifteen apps failing.
    public static var hasTap: Bool {
        let running = NSRunningApplication.runningApplications(withBundleIdentifier: bundleID)
        let pids = Set(running.map(\.processIdentifier))
        var count: UInt32 = 0
        guard CGGetEventTapList(0, nil, &count) == .success, count > 0 else { return false }
        var taps = [CGEventTapInformation](repeating: CGEventTapInformation(), count: Int(count))
        guard CGGetEventTapList(count, &taps, &count) == .success else { return false }
        return taps.prefix(Int(count)).contains { $0.enabled && pids.contains($0.tappingProcess) }
    }

    /// Aralo asks macOS every two seconds whether it may start its tap.
    public static func waitForTap(desktop: Desktop, timeout: TimeInterval = 10) throws {
        let deadline = desktop.now() + timeout
        while !hasTap {
            guard desktop.now() < deadline else { throw Problem.noTap }
            desktop.wait(0.2)
        }
    }

    /// Quits any Aralo that is running and starts the one at `app` on the
    /// matrix library, with `compatTable` in place of the built-in table.
    public static func restart(_ app: URL, library: URL, compatTable: URL?, desktop: Desktop) throws {
        guard FileManager.default.fileExists(atPath: app.path) else { throw Problem.notBuilt(app.path) }
        stop(desktop: desktop)

        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = false
        configuration.createsNewApplicationInstance = true
        configuration.environment = ["ARALO_LIBRARY": library.path, "ARALO_COMPAT": compatTable?.path ?? ""]
        // The first-run window would take the keyboard away from the app under test.
        configuration.arguments = ["-onboardingCompleted", "YES"]
        NSWorkspace.shared.openApplication(at: app, configuration: configuration)

        let deadline = desktop.now() + 15
        while !isRunning {
            guard desktop.now() < deadline else { throw Problem.didNotStart("not running after 15 seconds") }
            desktop.wait(0.2)
        }
        try waitForTap(desktop: desktop)
    }

    public static func stop(desktop: Desktop) {
        let running = NSRunningApplication.runningApplications(withBundleIdentifier: bundleID)
        running.forEach { $0.terminate() }
        let deadline = desktop.now() + 5
        while running.contains(where: { !$0.isTerminated }), desktop.now() < deadline {
            desktop.wait(0.2)
        }
        running.filter { !$0.isTerminated }.forEach { $0.forceTerminate() }
    }
}
