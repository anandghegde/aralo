import AppKit

/// What VoiceOver would find unnamed in a window: every control, image and
/// text box whose element has neither a label nor a title. The plan's
/// accessibility check (section 8, task 5.9) is this, run over each panel.
///
/// It reads the same tree VoiceOver reads, from inside the process: the
/// NSAccessibility elements a view and SwiftUI's hosting view hand out. So
/// nothing needs the Accessibility grant or UI automation to run it.
@MainActor
public enum AccessibilityAudit {
    /// One element VoiceOver would announce by its role alone.
    public struct Finding: Equatable, Sendable, CustomStringConvertible {
        public let role: String
        /// The named elements above it, outermost first, to find it by.
        public let path: [String]
        /// The SF Symbol name SwiftUI read out in place of a label, when it
        /// had no word of its own for the symbol.
        public var symbol: String?

        public var description: String {
            (path + [symbol.map { "\(role) \u{201C}\($0)\u{201D}" } ?? role]).joined(separator: " › ")
        }
    }

    /// SwiftUI names an image after its symbol when it has no word for it,
    /// and VoiceOver then reads out "exclamationmark dot arrow dot…".
    static func isSymbolName(_ name: String) -> Bool {
        name.contains(".") && !name.contains(" ")
            && name.allSatisfy { $0.isLowercase || $0.isNumber || $0 == "." }
    }

    /// The roles a user acts on or has to have read out. Groups, rows and
    /// static text name themselves by what they contain.
    static let named: Set<NSAccessibility.Role> = [
        .button, .checkBox, .radioButton, .popUpButton, .menuButton, .textField, .textArea,
        .slider, .image, .comboBox, .incrementor, .link, .disclosureTriangle, .colorWell
    ]

    /// Every element in the window that needs a name and has none. The window
    /// is shown off screen for the audit, because SwiftUI builds its elements
    /// only for a window that is on the screen list and an app that says an
    /// assistive client is listening.
    public static func unlabelled(in window: NSWindow) -> [Finding] {
        guard let content = window.contentView else { return [] }
        wakeAccessibility()
        if !window.isVisible {
            window.setFrameOrigin(NSPoint(x: -10_000, y: -10_000))
            window.orderBack(nil)
        }
        content.layoutSubtreeIfNeeded()
        // SwiftUI fills the tree in on the next turns of the run loop.
        RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.2))
        _ = content.accessibilityChildren()
        RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.2))
        var findings: [Finding] = []
        visit(content, path: [], depth: 0, into: &findings)
        return findings
    }

    /// What VoiceOver sets on an app when it starts: without it SwiftUI hands
    /// out no elements at all.
    private static func wakeAccessibility() {
        _ = NSApplication.shared
        NSApp.perform(
            NSSelectorFromString("accessibilitySetValue:forAttribute:"),
            with: NSNumber(value: true), with: "AXEnhancedUserInterface"
        )
    }

    /// The tree as text, one element a line, for a failing test to show.
    public static func outline(_ element: Any, depth: Int = 0) -> [String] {
        guard depth < 64, let node = element as? NSObject else { return [] }
        let role = attribute("accessibilityRole", of: node) as? String ?? "?"
        let text = name(of: node) ?? (attribute("accessibilityValue", of: node) as? String).map { "=\($0)" }
        let line = String(repeating: "  ", count: depth) + role + (text.map { " \u{201C}\($0)\u{201D}" } ?? "")
        let children = attribute("accessibilityChildren", of: node) as? [Any] ?? []
        return [line] + children.flatMap { outline($0, depth: depth + 1) }
    }

    /// What VoiceOver reads as the element's name. A text box's prompt is
    /// read out as its name when it is empty, which is how SwiftUI names a
    /// TextField by its title. A form row names its control by pointing it
    /// at the text beside it.
    private static func name(of node: NSObject) -> String? {
        let own = ["accessibilityLabel", "accessibilityTitle", "accessibilityPlaceholderValue"]
            .compactMap { attribute($0, of: node) as? String }
        let linked = (attribute("accessibilityTitleUIElement", of: node) as? NSObject).map { title in
            ["accessibilityLabel", "accessibilityValue"].compactMap { attribute($0, of: title) as? String }
        } ?? []
        return (own + linked).first { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    private static func visit(_ element: Any, path: [String], depth: Int, into findings: inout [Finding]) {
        // A cycle in a broken tree must not hang the test run.
        guard depth < 64, let node = element as? NSObject else { return }
        let role = (attribute("accessibilityRole", of: node) as? String).map(NSAccessibility.Role.init(rawValue:))
        // A scroll bar's arrows are AppKit's, named by the scroll bar.
        guard role != .scrollBar else { return }
        let name = name(of: node)
        if let role, named.contains(role), name.map(isSymbolName) ?? true, isElement(node) {
            findings.append(Finding(role: role.rawValue, path: path, symbol: name))
        }
        let below = name.map { path + [$0] } ?? path
        for child in attribute("accessibilityChildren", of: node) as? [Any] ?? [] {
            visit(child, path: below, depth: depth + 1, into: &findings)
        }
    }

    /// SwiftUI's elements answer the NSAccessibility getters without saying
    /// they conform to the protocol, so they are asked by selector.
    private static func attribute(_ getter: String, of node: NSObject) -> Any? {
        let selector = NSSelectorFromString(getter)
        guard node.responds(to: selector) else { return nil }
        return node.perform(selector)?.takeUnretainedValue()
    }

    private static func isElement(_ node: NSObject) -> Bool {
        guard node.responds(to: NSSelectorFromString("isAccessibilityElement")) else { return true }
        return (node.value(forKey: "accessibilityElement") as? Bool) ?? true
    }
}
