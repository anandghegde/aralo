import AraloBridge
import Carbon.HIToolbox
import CoreGraphics

/// A key-down event, reduced to what the classifier needs.
///
/// `text` is what the key produced under the current layout. It is at most a
/// few UTF-16 units, which Swift stores inline in the `String` value, so a
/// keystroke never reaches the heap on its way to the engine.
public struct KeyStroke: Equatable, Sendable {
    public var keyCode: CGKeyCode
    public var flags: CGEventFlags
    public var text: String

    public init(keyCode: CGKeyCode, flags: CGEventFlags = [], text: String) {
        self.keyCode = keyCode
        self.flags = flags
        self.text = text
    }
}

extension CGEventFlags: @retroactive @unchecked Sendable {}

/// What a key means to the engine.
public enum KeyClassification: Equatable, Sendable {
    /// Feed it to `Engine.onKey`.
    case input(KeyInput)
    /// The caret may have moved or the text may have changed behind our back:
    /// zero the buffer.
    case reset(ResetReason)
    /// Nothing happened yet (a dead key waiting for its letter).
    case ignore
}

public enum KeyClassifier {
    private static let navigationKeys: Set<Int> = [
        kVK_LeftArrow, kVK_RightArrow, kVK_UpArrow, kVK_DownArrow,
        kVK_Home, kVK_End, kVK_PageUp, kVK_PageDown,
        kVK_ForwardDelete, kVK_Escape, kVK_Help
    ]

    /// Function keys and arrows arrive as characters in this private-use range.
    private static let functionKeyScalars: ClosedRange<UInt32> = 0xF700...0xF8FF

    /// Whether the key can add text to the document, so that its text is
    /// worth translating. A shortcut, an arrow or Delete cannot.
    public static func typesText(_ stroke: KeyStroke) -> Bool {
        let keyCode = Int(stroke.keyCode)
        return stroke.flags.isDisjoint(with: [.maskCommand, .maskControl])
            && !navigationKeys.contains(keyCode) && keyCode != kVK_Delete
    }

    public static func classify(_ stroke: KeyStroke) -> KeyClassification {
        let flags = stroke.flags
        if flags.contains(.maskCommand) {
            // Compared by character, not key code, so Cmd+Z is found on every layout.
            let plainUndo = flags.isDisjoint(with: [.maskShift, .maskControl, .maskAlternate])
            return plainUndo && stroke.text.lowercased() == "z" ? .input(.undo) : .reset(.shortcut)
        }
        if flags.contains(.maskControl) {
            return .reset(.shortcut)
        }

        let keyCode = Int(stroke.keyCode)
        if navigationKeys.contains(keyCode) {
            return .reset(.navigation)
        }
        if keyCode == kVK_Delete {
            // Option+Delete removes a word, Fn+Delete deletes forwards: the
            // buffer can no longer say what is left of the caret.
            let plain = flags.isDisjoint(with: [.maskAlternate, .maskSecondaryFn])
            return plain ? .input(.backspace) : .reset(.navigation)
        }

        let scalars = stroke.text.unicodeScalars
        guard let scalar = scalars.first else {
            return .ignore
        }
        guard scalars.count == 1 else {
            return .reset(.unmappableInput)
        }
        switch scalar.value {
        case 0x03, 0x0A, 0x0D:
            // Return, and Enter on the keypad (which reports U+0003).
            return .input(.char(scalar: 0x0D))
        case 0x09:
            return .input(.char(scalar: 0x09))
        case 0x00..<0x20, 0x7F, functionKeyScalars:
            return .reset(.navigation)
        default:
            return .input(.char(scalar: scalar.value))
        }
    }
}
