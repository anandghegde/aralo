import AraloBridge
import Foundation

/// Runs expansion plans: the only code that posts events or touches the
/// pasteboard. Every call is synchronous and belongs on the injector queue.
///
/// `@unchecked Sendable` because that queue is the only place it is used:
/// it is created on one thread and handed to the serial queue for good.
public final class Injector: @unchecked Sendable {
    public struct Timing: Sendable {
        /// Pause after every synthetic key. Apps drop events that arrive faster.
        public var betweenKeys: TimeInterval = 0.001
        /// How long the target app gets to read the pasteboard before it is restored.
        public var pasteSettle: TimeInterval = 0.25
        /// Pause between a forwarded Cmd+Z and retyping the abbreviation.
        public var undoSettle: TimeInterval = 0.05

        public init() {}
    }

    /// Plan section 4.3: key events carry at most 20 UTF-16 units.
    public static let maxUnitsPerEvent = 20
    /// Longer text is pasted.
    public static let typingLimit = 120

    private let sink: EventSink
    private let pasteboard: PasteboardAccess
    private let timing: Timing
    private let sleep: (TimeInterval) -> Void

    public init(
        sink: EventSink,
        pasteboard: PasteboardAccess,
        timing: Timing = Timing(),
        sleep: @escaping (TimeInterval) -> Void = { Thread.sleep(forTimeInterval: $0) }
    ) {
        self.sink = sink
        self.pasteboard = pasteboard
        self.timing = timing
        self.sleep = sleep
    }

    /// Runs the steps in order. Returns how the text went in, which the engine
    /// needs to choose the undo technique.
    @discardableResult
    public func run(_ steps: [PlanStep]) -> InsertMethod {
        var method = InsertMethod.typed
        for step in steps {
            switch step {
            case .delete(let count):
                press(.backspace, times: count)
            case .insertText(let text):
                if insert(text) == .pasted {
                    method = .pasted
                }
            case .insertRich(_, let plain):
                // Rich text arrives with the rich-text milestone; until then
                // the plain rendering is better than nothing.
                if insert(plain) == .pasted {
                    method = .pasted
                }
            case .keyPress(let key):
                press(key == .return ? .return : .tab, times: 1)
            case .delay(let millis):
                sleep(TimeInterval(millis) / 1000)
            case .moveCursor(let graphemes, let select):
                press(.leftArrow(selecting: select), times: graphemes)
            }
        }
        return method
    }

    /// Takes an expansion back and puts the abbreviation where it was.
    public func undo(deleteCount: UInt32, retype: String, method: InsertMethod) {
        switch method {
        case .typed:
            press(.backspace, times: deleteCount)
        case .pasted:
            // A paste is one undo step in the app; Backspace would be hundreds.
            press(.undo, times: 1)
            sleep(timing.undoSettle)
        }
        type(retype)
    }

    /// Typing for short single-line text, paste for the rest. A line break in a
    /// typed event is a Return key to many apps, which sends the message.
    public static func method(for text: String) -> InsertMethod {
        let long = text.utf16.count > typingLimit
        return long || text.contains(where: \.isNewline) ? .pasted : .typed
    }

    /// Splits text into events of at most `maxUnits` UTF-16 units without ever
    /// cutting a user-perceived character in two.
    public static func chunks(of text: String, maxUnits: Int = maxUnitsPerEvent) -> [String] {
        var chunks: [String] = []
        var current = ""
        var units = 0
        for character in text {
            let size = character.utf16.count
            if units > 0, units + size > maxUnits {
                chunks.append(current)
                current = ""
                units = 0
            }
            current.append(character)
            units += size
        }
        if !current.isEmpty {
            chunks.append(current)
        }
        return chunks
    }

    private func insert(_ text: String) -> InsertMethod {
        let method = Self.method(for: text)
        switch method {
        case .typed: type(text)
        case .pasted: paste(text)
        }
        return method
    }

    private func type(_ text: String) {
        for chunk in Self.chunks(of: text) {
            press(.text(chunk), times: 1)
        }
    }

    private func paste(_ text: String) {
        let saved = pasteboard.contents()
        pasteboard.write(text: text, markerTypes: PasteboardMarker.all)
        let ours = pasteboard.changeCount
        press(.paste, times: 1)
        sleep(timing.pasteSettle)
        // If something else was copied while we waited, that is the user's
        // clipboard now. Leave it alone.
        if pasteboard.changeCount == ours {
            pasteboard.restore(saved)
        }
    }

    private func press(_ key: SyntheticKey, times: UInt32) {
        for _ in 0..<times {
            sink.post(key)
            sleep(timing.betweenKeys)
        }
    }
}
