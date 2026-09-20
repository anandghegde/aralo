import AppKit
import ApplicationServices

/// The few Accessibility calls the harness makes.
@MainActor
enum Accessibility {
    static func frontmostPID() -> pid_t? {
        let system = AXUIElementCreateSystemWide()
        guard let app = element(system, kAXFocusedApplicationAttribute) else { return nil }
        var pid: pid_t = 0
        return AXUIElementGetPid(app, &pid) == .success ? pid : nil
    }

    static func focusedText(ofAppWithPID pid: pid_t) -> String? {
        let app = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(app, 1)
        guard let focused = element(app, kAXFocusedUIElementAttribute) else { return nil }
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(focused, kAXValueAttribute as CFString, &value) == .success else {
            return nil
        }
        return value as? String
    }

    /// Chromium and Electron build their Accessibility tree only once
    /// something asks for it. These are the two attributes that ask. An app
    /// that knows neither refuses, which is fine.
    static func wakeTree(ofAppWithBundleID bundleID: String) {
        for running in NSRunningApplication.runningApplications(withBundleIdentifier: bundleID) {
            let app = AXUIElementCreateApplication(running.processIdentifier)
            AXUIElementSetAttributeValue(app, "AXManualAccessibility" as CFString, kCFBooleanTrue)
            AXUIElementSetAttributeValue(app, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
        }
    }

    private static func element(_ parent: AXUIElement, _ attribute: String) -> AXUIElement? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(parent, attribute as CFString, &value) == .success,
              let value, CFGetTypeID(value) == AXUIElementGetTypeID()
        else { return nil }
        // The type ID was checked on the line above.
        return unsafeDowncast(value, to: AXUIElement.self)
    }
}
