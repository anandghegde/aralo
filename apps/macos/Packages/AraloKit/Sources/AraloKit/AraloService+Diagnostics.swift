import AraloBridge
import Foundation

// MARK: - Local counters and the diagnostics report (plan 5.7)

extension AraloService {
    /// The diagnostics report: versions, permissions, the library's size, the
    /// AI settings in outline and the local counters. The core builds it, and
    /// it holds no snippet, typed text, context, file name or key, so a user
    /// can paste it into an issue as it is. Nil before the library opened.
    public func diagnosticsReport() -> String? {
        guard let core else { return nil }
        let permissions = Permissions.status
        let facts = DiagnosticFacts(
            appVersion: Self.appVersion,
            osVersion: Self.osVersion,
            accessibility: permissions.accessibility,
            inputMonitoring: permissions.inputMonitoring,
            tapRunning: isTapRunning,
            secureInput: secureInput?.isSecureInputOn,
            paused: isPaused,
            // The shell keeps no list of its own yet: only the presets.
            excludedApps: nil
        )
        // The settings as they already are, if something opened them. The
        // report does not open profiles.toml or the keychain for itself.
        return core.diagnosticsReport(facts: facts, ai: aiProfiles)
    }

    /// Puts the report on the pasteboard, as plain text.
    public func copyDiagnostics() {
        guard let report = diagnosticsReport() else { return }
        pasteboard.write(text: report, markerTypes: [])
    }

    /// Writes the counters now. For the app quitting.
    public func saveCounters() {
        core?.saveCounters()
    }

    static var appVersion: String {
        let info = Bundle.main.infoDictionary ?? [:]
        let version = info["CFBundleShortVersionString"] as? String ?? "unknown"
        let build = info["CFBundleVersion"] as? String ?? "unknown"
        return "Aralo \(version) (\(build))"
    }

    static var osVersion: String {
        let version = ProcessInfo.processInfo.operatingSystemVersion
        return "macOS \(version.majorVersion).\(version.minorVersion).\(version.patchVersion)"
    }
}

extension EventTap.Health {
    /// The counter this goes under.
    var shellEvent: ShellEvent {
        switch self {
        case .timedOut: .tapTimedOut
        case .disabledByUserInput: .tapDisabledByUser
        case .reenabled: .tapReenabled
        }
    }
}
