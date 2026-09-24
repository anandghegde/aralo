import Carbon.HIToolbox
import CoreGraphics

/// One synthetic key press (down, then up).
public struct SyntheticKey: Equatable, Sendable {
    public var keyCode: CGKeyCode
    public var flags: CGEventFlags
    /// When set, the event carries this text instead of what `keyCode` would
    /// produce. At most `Injector.maxUnitsPerEvent` UTF-16 units.
    public var text: String?
    /// For a shortcut: the character that names it. The sink presses the key
    /// that gives this character in the user's keyboard layout, and `keyCode`
    /// (the US key) only when the layout has none.
    public var equivalent: Character?

    public init(keyCode: CGKeyCode, flags: CGEventFlags = [], text: String? = nil, equivalent: Character? = nil) {
        self.keyCode = keyCode
        self.flags = flags
        self.text = text
        self.equivalent = equivalent
    }

    public static let backspace = SyntheticKey(keyCode: CGKeyCode(kVK_Delete))
    public static let copy = SyntheticKey(keyCode: CGKeyCode(kVK_ANSI_C), flags: .maskCommand, equivalent: "c")
    public static let paste = SyntheticKey(keyCode: CGKeyCode(kVK_ANSI_V), flags: .maskCommand, equivalent: "v")
    public static let undo = SyntheticKey(keyCode: CGKeyCode(kVK_ANSI_Z), flags: .maskCommand, equivalent: "z")
    public static let `return` = SyntheticKey(keyCode: CGKeyCode(kVK_Return))
    public static let tab = SyntheticKey(keyCode: CGKeyCode(kVK_Tab))

    public static func leftArrow(selecting: Bool) -> SyntheticKey {
        SyntheticKey(keyCode: CGKeyCode(kVK_LeftArrow), flags: selecting ? .maskShift : [])
    }

    public static func text(_ text: String) -> SyntheticKey {
        SyntheticKey(keyCode: 0, text: text)
    }
}

/// Where synthetic keys go. The injector is the only caller; tests use a fake.
public protocol EventSink: AnyObject {
    func post(_ key: SyntheticKey)
}

/// Posts real events, each tagged so that the tap lets it through unread.
public final class SystemEventSink: EventSink {
    /// "ARALO" in ASCII, in the event's source user-data field.
    public static let selfEventTag: Int64 = 0x41_52_41_4C_4F

    private let source: CGEventSource?
    private let shortcuts: ShortcutKeyCodes

    public init(shortcuts: ShortcutKeyCodes = ShortcutKeyCodes()) {
        self.shortcuts = shortcuts
        // A private source keeps the user's physically held modifiers (the
        // Shift of "TY ") out of the events we post.
        source = CGEventSource(stateID: .privateState)
        // By default the system may drop hardware keys for a quarter of a
        // second after a synthetic event. The user is still typing.
        source?.localEventsSuppressionInterval = 0
    }

    /// The physical key to press for `key` in the user's keyboard layout.
    public static func keyCode(for key: SyntheticKey, shortcuts: ShortcutKeyCodes) -> CGKeyCode {
        key.equivalent.flatMap(shortcuts.keyCode(for:)) ?? key.keyCode
    }

    public func post(_ key: SyntheticKey) {
        let keyCode = Self.keyCode(for: key, shortcuts: shortcuts)
        for keyDown in [true, false] {
            guard let event = CGEvent(keyboardEventSource: source, virtualKey: keyCode, keyDown: keyDown) else {
                continue
            }
            event.flags = key.flags
            if let text = key.text {
                let units = Array(text.utf16)
                event.keyboardSetUnicodeString(stringLength: units.count, unicodeString: units)
            }
            event.setIntegerValueField(.eventSourceUserData, value: Self.selfEventTag)
            event.post(tap: .cghidEventTap)
        }
    }
}
