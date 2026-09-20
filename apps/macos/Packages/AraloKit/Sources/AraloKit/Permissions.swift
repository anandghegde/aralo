import AppKit
import ApplicationServices
import CoreGraphics

/// The two grants the tap needs, and where the user gives them.
public enum Permissions {
    public struct Status: Equatable, Sendable {
        /// Posting keys and consuming events.
        public var accessibility: Bool
        /// Seeing key events from other apps.
        public var inputMonitoring: Bool

        public var allGranted: Bool { accessibility && inputMonitoring }
    }

    public static var status: Status {
        Status(accessibility: AXIsProcessTrusted(), inputMonitoring: CGPreflightListenEventAccess())
    }

    /// Shows the system prompts for whatever is missing. Call only after the
    /// app has explained what it reads and what it never stores.
    public static func request() {
        if !AXIsProcessTrusted() {
            // The value of kAXTrustedCheckOptionPrompt, spelled out because the
            // global is not concurrency-safe to import.
            let options = ["AXTrustedCheckOptionPrompt": true] as CFDictionary
            _ = AXIsProcessTrustedWithOptions(options)
        }
        if !CGPreflightListenEventAccess() {
            _ = CGRequestListenEventAccess()
        }
    }

    public enum Pane: String {
        case accessibility = "Privacy_Accessibility"
        case inputMonitoring = "Privacy_ListenEvent"
    }

    @MainActor
    public static func openSettings(_ pane: Pane) {
        let address = "x-apple.systempreferences:com.apple.preference.security?\(pane.rawValue)"
        if let url = URL(string: address) {
            NSWorkspace.shared.open(url)
        }
    }
}
