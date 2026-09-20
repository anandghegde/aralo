import Carbon.HIToolbox
import CoreGraphics
import Foundation

/// What a key types, tracked across dead keys.
///
/// The string on a key event is the key on its own: after Option+E the "e"
/// arrives as "e", and the app's text system is what turns the pair into "é".
/// The engine has to see what the document gets, so the tap runs the same
/// translation itself, `UCKeyTranslate` with the dead-key state kept from one
/// key to the next.
///
/// The layout is written on the main thread, where Text Input Sources calls
/// are safe, and read on the tap thread.
public final class KeyTranslator: @unchecked Sendable {
    /// A keyboard layout's `uchr` data and the keyboard it applies to.
    public struct Layout: Equatable, Sendable {
        public var data: Data
        public var keyboardType: UInt32

        public init(data: Data, keyboardType: UInt32) {
            self.data = data
            self.keyboardType = keyboardType
        }
    }

    private let lock = NSLock()
    private var layout: Layout?
    private var composing = false
    /// Opaque to us. Not zero after a finished pair either, so it cannot say
    /// whether a dead key is waiting; `pending` does.
    private var deadKeyState: UInt32 = 0
    private var pending = false

    public init(layout: Layout? = nil, composing: Bool = false) {
        self.layout = layout
        self.composing = composing
    }

    /// True while an input method (Japanese, Chinese, …) turns keys into text
    /// in a composition window. What reaches the document is then unknown, so
    /// matching pauses.
    public var isComposing: Bool {
        lock.withLock { composing }
    }

    public var isDeadKeyPending: Bool {
        lock.withLock { pending }
    }

    /// The input source changed.
    public func replace(layout: Layout?, composing: Bool) {
        lock.withLock {
            self.layout = layout
            self.composing = composing
            deadKeyState = 0
            pending = false
        }
    }

    /// Forgets a pending dead key: the user clicked, moved or pressed a
    /// shortcut, and the app dropped its half of the pair too. Returns whether
    /// there was one.
    @discardableResult
    public func clear() -> Bool {
        lock.withLock {
            defer {
                deadKeyState = 0
                pending = false
            }
            return pending
        }
    }

    /// The text this key adds to the document. Empty for a dead key waiting
    /// for its letter. Nil when there is no layout to ask (the caller falls
    /// back to the string on the event).
    public func text(keyCode: CGKeyCode, flags: CGEventFlags) -> String? {
        lock.withLock {
            guard let layout else { return nil }
            return layout.data.withUnsafeBytes { bytes -> String? in
                guard let keyboard = bytes.bindMemory(to: UCKeyboardLayout.self).baseAddress else { return nil }
                var length = 0
                var units = [UniChar](repeating: 0, count: 8)
                // The stack copy held a keystroke; do not leave it behind.
                defer { for index in units.indices { units[index] = 0 } }
                let status = UCKeyTranslate(
                    keyboard, keyCode, UInt16(kUCKeyActionDown), Self.modifierState(flags),
                    layout.keyboardType, 0, &deadKeyState, units.count, &length, &units
                )
                guard status == noErr else {
                    deadKeyState = 0
                    pending = false
                    return nil
                }
                pending = length == 0 && deadKeyState != 0
                return String(utf16CodeUnits: units, count: min(length, units.count))
            }
        }
    }

    /// `UCKeyTranslate` wants the Carbon modifier bits, shifted down a byte.
    static func modifierState(_ flags: CGEventFlags) -> UInt32 {
        var carbon = 0
        if flags.contains(.maskShift) { carbon |= shiftKey }
        if flags.contains(.maskAlphaShift) { carbon |= alphaLock }
        if flags.contains(.maskAlternate) { carbon |= optionKey }
        if flags.contains(.maskControl) { carbon |= controlKey }
        if flags.contains(.maskCommand) { carbon |= cmdKey }
        return UInt32(carbon >> 8) & 0xFF
    }
}
