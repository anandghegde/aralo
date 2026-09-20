import AraloBridge
import Foundation

/// Runs expansion plans: the only code that posts events or touches the
/// pasteboard. Every call is synchronous and belongs on the injector queue.
///
/// How it inserts, deletes and undoes is not decided here. Each plan arrives
/// with the front app's `InjectionProfile`, its row of the compatibility table
/// (`data/compat/apps.toml`), and the injector follows it.
///
/// `@unchecked Sendable` because that queue is the only place it is used:
/// it is created on one thread and handed to the serial queue for good.
public final class Injector: @unchecked Sendable {
    /// What `run` did, as far as the engine's undo record needs to know.
    public struct Outcome: Equatable, Sendable {
        /// How the text went in, which chooses the undo technique.
        public var method: InsertMethod
        /// False when a line break went in as a Return key. The app may have
        /// acted on it (a shell ran the line), so Backspace cannot take the
        /// expansion back and undo must not be offered.
        public var undoable: Bool
    }

    /// Plan section 4.3: key events carry at most 20 UTF-16 units.
    public static let maxUnitsPerEvent = 20
    /// Pause between a forwarded Cmd+Z and retyping the abbreviation.
    public static let undoSettle: TimeInterval = 0.05

    private let sink: EventSink
    private let pasteboard: PasteboardAccess
    private let sleep: (TimeInterval) -> Void

    public init(
        sink: EventSink,
        pasteboard: PasteboardAccess,
        sleep: @escaping (TimeInterval) -> Void = { Thread.sleep(forTimeInterval: $0) }
    ) {
        self.sink = sink
        self.pasteboard = pasteboard
        self.sleep = sleep
    }

    /// Runs the steps in order, the way `profile` says.
    @discardableResult
    public func run(_ steps: [PlanStep], profile: InjectionProfile) -> Outcome {
        var outcome = Outcome(method: .typed, undoable: true)
        for step in steps {
            switch step {
            case .delete(let count):
                delete(count, profile)
            case .insertText(let text):
                insert(text, profile, &outcome)
            case .insertRich(_, let plain):
                // Rich text arrives with the rich-text milestone; until then
                // the plain rendering is better than nothing.
                insert(plain, profile, &outcome)
            case .keyPress(let key):
                press(key == .return ? .return : .tab, times: 1, profile)
            case .delay(let millis):
                sleep(TimeInterval(millis) / 1000)
            case .moveCursor(let graphemes, let select):
                press(.leftArrow(selecting: select), times: graphemes, profile)
            }
        }
        return outcome
    }

    /// Takes an expansion back and puts the abbreviation where it was.
    public func undo(deleteCount: UInt32, retype: String, method: InsertMethod, profile: InjectionProfile) {
        if method == .pasted, profile.undo == .native {
            // A paste is one undo step in the app; Backspace would be hundreds.
            press(.undo, times: 1, profile)
            sleep(Self.undoSettle)
        } else {
            delete(deleteCount, profile)
        }
        type(retype, profile)
    }

    /// `auto` types short single-line text and pastes the rest: a line break
    /// in a typed event is a Return key to many apps, which sends the message.
    public static func method(for text: String, profile: InjectionProfile) -> InsertMethod {
        switch profile.insert {
        case .type:
            return .typed
        case .paste:
            return .pasted
        case .auto:
            let long = text.utf16.count > Int(profile.typingLimit)
            return long || text.contains(where: \.isNewline) ? .pasted : .typed
        }
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

    private func insert(_ text: String, _ profile: InjectionProfile, _ outcome: inout Outcome) {
        guard !text.isEmpty else { return }
        switch Self.method(for: text, profile: profile) {
        case .typed:
            if type(text, profile) {
                outcome.undoable = false
            }
        case .pasted:
            paste(text, profile)
            outcome.method = .pasted
        }
    }

    /// Types the text, a line break as a Return key. Returns whether there was one.
    @discardableResult
    private func type(_ text: String, _ profile: InjectionProfile) -> Bool {
        var pressedReturn = false
        for (index, line) in text.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline).enumerated() {
            if index > 0 {
                press(.return, times: 1, profile)
                pressedReturn = true
            }
            for chunk in Self.chunks(of: String(line)) {
                press(.text(chunk), times: 1, profile)
            }
        }
        return pressedReturn
    }

    private func delete(_ count: UInt32, _ profile: InjectionProfile) {
        guard count > 0 else { return }
        switch profile.delete {
        case .backspace:
            press(.backspace, times: count, profile)
        case .select:
            press(.leftArrow(selecting: true), times: count, profile)
            press(.backspace, times: 1, profile)
        }
    }

    private func paste(_ text: String, _ profile: InjectionProfile) {
        let saved = pasteboard.contents()
        pasteboard.write(text: text, markerTypes: PasteboardMarker.all)
        let ours = pasteboard.changeCount
        press(.paste, times: 1, profile)
        sleep(TimeInterval(profile.pasteSettleMs) / 1000)
        // If something else was copied while we waited, that is the user's
        // clipboard now. Leave it alone.
        if pasteboard.changeCount == ours {
            pasteboard.restore(saved)
        }
    }

    /// Apps drop events that arrive faster than they read them, hence the pause.
    private func press(_ key: SyntheticKey, times: UInt32, _ profile: InjectionProfile) {
        let pause = TimeInterval(profile.keyDelayMs) / 1000
        for _ in 0..<times {
            sink.post(key)
            sleep(pause)
        }
    }
}
