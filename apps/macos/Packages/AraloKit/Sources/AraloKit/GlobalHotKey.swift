import AppKit
import Carbon.HIToolbox

/// A system-wide keyboard shortcut, through the Carbon hot-key API: the one
/// way to get a key in every app that needs no permission, so pausing works
/// before Accessibility is granted and does not depend on the event tap.
@MainActor
public final class GlobalHotKey {
    /// A shortcut named by the character on the key, so that it is the key
    /// labelled "P" on every keyboard layout.
    public struct Shortcut: Equatable, Sendable {
        public var character: Character
        public var modifiers: NSEvent.ModifierFlags

        public init(character: Character, modifiers: NSEvent.ModifierFlags) {
            self.character = character
            self.modifiers = modifiers
        }

        /// Control+Option+Command+P. Three modifiers keep it clear of the
        /// shortcuts apps define for themselves.
        public static let pause = Shortcut(character: "p", modifiers: [.control, .option, .command])

        /// As menus write it: the modifiers in the system's order, then the key.
        public var display: String {
            let symbols: [(NSEvent.ModifierFlags, String)] = [
                (.control, "⌃"), (.option, "⌥"), (.shift, "⇧"), (.command, "⌘")
            ]
            let held = symbols.filter { modifiers.contains($0.0) }.map(\.1).joined()
            return held + String(character).uppercased()
        }
    }

    private static let signature: OSType = 0x4152_4C4F // "ARLO"
    private static var nextID: UInt32 = 1

    private let id: UInt32
    private let action: @MainActor () -> Void
    // Only touched on the main actor, and in `deinit` when nothing else can be.
    private nonisolated(unsafe) var hotKey: EventHotKeyRef?
    private nonisolated(unsafe) var handler: EventHandlerRef?

    public init(action: @escaping @MainActor () -> Void) {
        id = Self.nextID
        Self.nextID += 1
        self.action = action
    }

    public var isRegistered: Bool { hotKey != nil }

    /// Replaces the shortcut. False when the system refuses it, which is what
    /// happens when another app already owns the combination.
    @discardableResult
    public func register(keyCode: CGKeyCode, modifiers: NSEvent.ModifierFlags) -> Bool {
        unregister()
        guard installHandler() else { return false }
        let status = RegisterEventHotKey(
            UInt32(keyCode), Self.carbonModifiers(modifiers), EventHotKeyID(signature: Self.signature, id: id),
            GetEventDispatcherTarget(), 0, &hotKey
        )
        if status != noErr { hotKey = nil }
        return status == noErr
    }

    public func unregister() {
        if let hotKey { UnregisterEventHotKey(hotKey) }
        hotKey = nil
    }

    static func carbonModifiers(_ flags: NSEvent.ModifierFlags) -> UInt32 {
        var carbon = 0
        if flags.contains(.command) { carbon |= cmdKey }
        if flags.contains(.option) { carbon |= optionKey }
        if flags.contains(.control) { carbon |= controlKey }
        if flags.contains(.shift) { carbon |= shiftKey }
        return UInt32(carbon)
    }

    /// One handler per hot key, installed once and kept: every handler sees
    /// every hot key of the process, so each checks the ID.
    private func installHandler() -> Bool {
        guard handler == nil else { return true }
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let status = InstallEventHandler(
            GetEventDispatcherTarget(), hotKeyPressed, 1, &spec, Unmanaged.passUnretained(self).toOpaque(), &handler
        )
        return status == noErr
    }

    fileprivate func pressed(_ pressedID: EventHotKeyID) {
        if pressedID.signature == Self.signature, pressedID.id == id {
            action()
        }
    }

    deinit {
        // Carbon holds an unretained pointer to this object.
        if let hotKey { UnregisterEventHotKey(hotKey) }
        if let handler { RemoveEventHandler(handler) }
    }
}

private func hotKeyPressed(_: EventHandlerCallRef?, event: EventRef?, userData: UnsafeMutableRawPointer?) -> OSStatus {
    guard let event, let userData else { return OSStatus(eventNotHandledErr) }
    var pressedID = EventHotKeyID()
    let status = GetEventParameter(
        event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID), nil,
        MemoryLayout<EventHotKeyID>.size, nil, &pressedID
    )
    guard status == noErr else { return status }
    // Carbon delivers hot keys on the main run loop.
    let address = UInt(bitPattern: userData)
    MainActor.assumeIsolated {
        guard let pointer = UnsafeRawPointer(bitPattern: address) else { return }
        Unmanaged<GlobalHotKey>.fromOpaque(pointer).takeUnretainedValue().pressed(pressedID)
    }
    return noErr
}
