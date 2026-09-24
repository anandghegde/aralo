import AppKit
import AraloBridge
import Foundation

/// What Accessibility says is selected in an app (PRD A3).
///
/// Reading it this way touches nothing: no key is posted and the clipboard is
/// left alone. Plenty of apps do not answer, and for those the selection is
/// copied instead.
public enum AccessibilitySelection: Equatable, Sendable {
    case text(String)
    /// A text field or text area says nothing in it is selected. Only an
    /// element that is plainly text is believed on this: copying with
    /// nothing selected copies the whole line in some editors.
    case nothing
    /// The keyboard is in a password field. Its text is never read.
    case passwordField
    /// No answer, or not one to trust. Copying is the way to find out.
    case unavailable

    /// How long an app gets to answer each question. An app that is hung does
    /// not get to hang the hot key with it.
    public static let timeout: Float = 0.25

    /// Asks the process `pid` what is selected in the element with the
    /// keyboard.
    public static func read(pid: pid_t) -> AccessibilitySelection {
        let app = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(app, timeout)
        guard let element = focusedElement(of: app) else { return .unavailable }
        AXUIElementSetMessagingTimeout(element, timeout)
        let subrole: String? = attribute(kAXSubroleAttribute, of: element)
        if subrole == kAXSecureTextFieldSubrole as String {
            return .passwordField
        }
        guard let selected: String = attribute(kAXSelectedTextAttribute, of: element) else {
            return .unavailable
        }
        if !selected.isEmpty {
            return .text(selected)
        }
        let role: String? = attribute(kAXRoleAttribute, of: element)
        let textRoles = [kAXTextFieldRole as String, kAXTextAreaRole as String, kAXComboBoxRole as String]
        return role.map(textRoles.contains) == true ? .nothing : .unavailable
    }

    /// The title of the window with the keyboard in the process `pid`: what a
    /// snippet whose AI block declared `window` sends. Nil when the app does
    /// not say.
    public static func windowTitle(pid: pid_t) -> String? {
        let app = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(app, timeout)
        var value: CFTypeRef?
        let status = AXUIElementCopyAttributeValue(app, kAXFocusedWindowAttribute as CFString, &value)
        guard status == .success, let value, CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
        let window = value as! AXUIElement // swiftlint:disable:this force_cast
        AXUIElementSetMessagingTimeout(window, timeout)
        let title: String? = attribute(kAXTitleAttribute, of: window)
        return title?.isEmpty == false ? title : nil
    }

    private static func focusedElement(of app: AXUIElement) -> AXUIElement? {
        var value: CFTypeRef?
        let status = AXUIElementCopyAttributeValue(app, kAXFocusedUIElementAttribute as CFString, &value)
        guard status == .success, let value, CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
        return (value as! AXUIElement) // swiftlint:disable:this force_cast
    }

    private static func attribute<Value>(_ name: String, of element: AXUIElement) -> Value? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
        return value as? Value
    }
}

/// Why a command could not read the selection, or could not put its answer
/// in place of it.
public enum SelectionFailure: Error, Equatable, Sendable {
    /// The core leaves this app alone: Aralo is paused, or the app is one it
    /// never types into.
    case refused(InsertRefusal)
    /// The app the command was asked for over could not be brought back.
    case noTargetApp
    /// The keyboard is in a password field, or an app has secure input on.
    case passwordField
    /// There is no selection to work on.
    case nothingSelected
}

/// The selection in the app a command was asked for over: reading it, and
/// putting the answer in its place.
///
/// A protocol because the command panel's model is the same whatever reads
/// and writes the app; the tests wire in one that writes down what it did.
@MainActor
public protocol SelectionAccess {
    /// What is selected, or why it cannot be read.
    func read() async -> Result<String, SelectionFailure>
    /// Replaces the selection with `text`. Nil when it has gone in.
    func replace(with text: String) async -> SelectionFailure?
}

/// Reads the selection through Accessibility and copies it when that says
/// nothing useful; replaces it by pasting, so that one Cmd+Z in the app
/// brings the original back.
///
/// Both ends ask the core first. A paused Aralo or an excluded app reads
/// nothing and replaces nothing, the same as it expands nothing.
@MainActor
public final class SelectionCapture: SelectionAccess {
    private let controller: ExpansionController
    private let target: TargetApp
    private let secureInput: @MainActor () -> Bool
    private let accessibility: @MainActor (TargetApp) -> AccessibilitySelection

    public init(
        controller: ExpansionController,
        target: TargetApp,
        secureInput: @escaping @MainActor () -> Bool,
        accessibility: @escaping @MainActor (TargetApp) -> AccessibilitySelection = SelectionCapture.readAccessibility
    ) {
        self.controller = controller
        self.target = target
        self.secureInput = secureInput
        self.accessibility = accessibility
    }

    /// The real reader, for an app macOS says is running.
    public static func readAccessibility(_ target: TargetApp) -> AccessibilitySelection {
        guard let running = target as? RunningTargetApp else { return .unavailable }
        return AccessibilitySelection.read(pid: running.processIdentifier)
    }

    public func read() async -> Result<String, SelectionFailure> {
        let profile: InjectionProfile
        switch permitted() {
        case .success(let allowed): profile = allowed
        case .failure(let failure): return .failure(failure)
        }
        switch accessibility(target) {
        case .text(let text):
            return .success(text)
        case .nothing:
            return .failure(.nothingSelected)
        case .passwordField:
            return .failure(.passwordField)
        case .unavailable:
            guard let copied = await controller.copySelection(profile: profile), !copied.isEmpty else {
                return .failure(.nothingSelected)
            }
            return .success(copied)
        }
    }

    /// The panel must have given the keyboard back first: the paste goes
    /// wherever the keyboard is.
    public func replace(with text: String) async -> SelectionFailure? {
        guard await target.activate() else { return .noTargetApp }
        // Asked again: Aralo may have been paused while the answer was read,
        // or a password field may have taken the keyboard.
        let profile: InjectionProfile
        switch permitted() {
        case .success(let allowed): profile = allowed
        case .failure(let failure): return failure
        }
        await controller.replaceSelection(with: text, profile: profile)
        return nil
    }

    private func permitted() -> Result<InjectionProfile, SelectionFailure> {
        if secureInput() {
            return .failure(.passwordField)
        }
        switch controller.commandTarget(app: target.bundleId) {
        case .ready(let profile):
            return .success(profile)
        case .refused(let reason):
            return .failure(.refused(reason))
        }
    }
}
