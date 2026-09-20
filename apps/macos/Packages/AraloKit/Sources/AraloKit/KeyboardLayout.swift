import Carbon.HIToolbox
import CoreGraphics
import Foundation

/// Key codes for the shortcuts Aralo posts, in the user's keyboard layout.
///
/// A key code names a physical key, and an app decides what Cmd+key means from
/// the character the layout gives that key. On Dvorak the key that types "v"
/// sits where "." is on a US keyboard, so the US key code for V would not paste.
///
/// Written on the main thread, read on the injector queue.
public final class ShortcutKeyCodes: @unchecked Sendable {
    private let lock = NSLock()
    private var codes: [Character: CGKeyCode]

    public init(_ codes: [Character: CGKeyCode] = [:]) {
        self.codes = codes
    }

    public func keyCode(for character: Character) -> CGKeyCode? {
        lock.withLock { codes[character] }
    }

    public func replace(with codes: [Character: CGKeyCode]) {
        lock.withLock { self.codes = codes }
    }
}

public enum KeyboardLayout {
    /// The characters of every shortcut the injector posts.
    public static let shortcutCharacters: Set<Character> = ["v", "z"]

    /// What one key produces, with and without Command held.
    public typealias Translate = (_ keyCode: CGKeyCode, _ command: Bool) -> String?

    /// The key to press with Command for each character. What the key gives
    /// while Command is held wins, because that is what apps compare against:
    /// "Dvorak – QWERTY ⌘" types Dvorak and keeps its shortcuts on the QWERTY
    /// keys. A character no key produces is left out, and the caller falls back
    /// to the US key.
    public static func keyCodes(for characters: Set<Character>, translate: Translate) -> [Character: CGKeyCode] {
        var found: [Character: CGKeyCode] = [:]
        for command in [true, false] {
            for keyCode in CGKeyCode(0)..<128 {
                guard let text = translate(keyCode, command), text.count == 1, let character = text.first,
                      characters.contains(character), found[character] == nil
                else { continue }
                found[character] = keyCode
            }
        }
        return found
    }

    /// The keyboard layout in use now. With an input method selected, this is
    /// the layout underneath it. Text Input Sources calls are only safe on the
    /// main thread.
    @MainActor
    public static func currentLayout() -> KeyTranslator.Layout? {
        guard let source = TISCopyCurrentKeyboardLayoutInputSource()?.takeRetainedValue() else { return nil }
        return layout(of: source)
    }

    /// Nil for a layout with no Unicode data, which only old third-party
    /// layouts are.
    @MainActor
    static func layout(of source: TISInputSource) -> KeyTranslator.Layout? {
        guard let property = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData) else { return nil }
        let data = Unmanaged<CFData>.fromOpaque(property).takeUnretainedValue() as Data
        return KeyTranslator.Layout(data: data, keyboardType: UInt32(LMGetKbdType()))
    }

    /// Whether the selected input source composes text in a window of its own
    /// before the document gets any: Japanese, Chinese, Korean and the like.
    /// A plain layout does not, and neither does an input method's Roman mode,
    /// which types the letters on the keys.
    @MainActor
    public static func currentInputSourceComposes() -> Bool {
        guard let source = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue() else { return false }
        return composes(type: string(kTISPropertyInputSourceType, of: source),
                        inputMode: string(kTISPropertyInputModeID, of: source))
    }

    static func composes(type: String?, inputMode: String?) -> Bool {
        guard let type, type != kTISTypeKeyboardLayout as String else { return false }
        return inputMode != romanInputMode
    }

    /// `kTextServiceInputModeRoman`.
    static let romanInputMode = "com.apple.inputmethod.Roman"

    @MainActor
    private static func string(_ key: CFString, of source: TISInputSource) -> String? {
        guard let property = TISGetInputSourceProperty(source, key) else { return nil }
        return Unmanaged<CFString>.fromOpaque(property).takeUnretainedValue() as String
    }

    /// The same, for the keyboard layout in use now.
    @MainActor
    public static func currentKeyCodes(for characters: Set<Character> = shortcutCharacters) -> [Character: CGKeyCode] {
        guard let current = currentLayout() else { return [:] }
        let keyboardType = current.keyboardType
        return current.data.withUnsafeBytes { bytes -> [Character: CGKeyCode] in
            guard let layout = bytes.bindMemory(to: UCKeyboardLayout.self).baseAddress else { return [:] }
            return keyCodes(for: characters) { keyCode, command in
                var deadKeys: UInt32 = 0
                var length = 0
                var units = [UniChar](repeating: 0, count: 4)
                let status = UCKeyTranslate(
                    layout, keyCode, UInt16(kUCKeyActionDown), command ? UInt32(cmdKey >> 8) & 0xFF : 0,
                    keyboardType, OptionBits(kUCKeyTranslateNoDeadKeysMask), &deadKeys, units.count, &length, &units
                )
                guard status == noErr, length > 0 else { return nil }
                return String(utf16CodeUnits: units, count: length)
            }
        }
    }
}
