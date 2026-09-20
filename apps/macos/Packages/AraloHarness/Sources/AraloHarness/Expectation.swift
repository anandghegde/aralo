import AraloBridge
import AraloKit
import Foundation

/// What one case should leave in the field, as the core itself says it: the
/// harness never keeps a second copy of a snippet body.
public struct Expectation: Equatable, Sendable {
    /// The abbreviation and the delimiter, as typed.
    public var typed: String
    /// The text the expansion inserts.
    public var text: String
    /// Characters between the caret and the end of `text` afterwards.
    public var caretBack: Int
    public var method: InsertMethod
    public var undoable: Bool

    public init(typed: String, text: String, caretBack: Int = 0, method: InsertMethod, undoable: Bool = true) {
        self.typed = typed
        self.text = text
        self.caretBack = caretBack
        self.method = method
        self.undoable = undoable
    }

    /// `text` with `marker` typed where the caret was left.
    public func text(withAtCaret marker: String) -> String {
        var result = text
        let index = result.index(result.endIndex, offsetBy: -min(caretBack, result.count))
        result.insert(contentsOf: marker, at: index)
        return result
    }

    public enum Problem: Error, Equatable, CustomStringConvertible {
        case noExpansion(String)

        public var description: String {
            switch self {
            case .noExpansion(let abbreviation):
                "the library does not expand \"\(abbreviation)\"; is it fixtures/matrix/library?"
            }
        }
    }

    public static let delimiter = " "

    /// Types the abbreviation into `engine` and reads the plan that comes back.
    public static func ask(_ engine: Engine, abbreviation: String) throws -> Expectation {
        engine.reset(reason: .manual)
        let typed = abbreviation + delimiter
        var answer: Expectation?
        for scalar in typed.unicodeScalars {
            guard case .expand(_, _, let steps, let undoDeleteCount, let profile) =
                engine.onKey(key: .char(scalar: scalar.value))
            else { continue }
            answer = read(steps, profile: profile, typed: typed, undoable: undoDeleteCount != nil)
        }
        engine.reset(reason: .manual)
        guard let answer else { throw Problem.noExpansion(abbreviation) }
        return answer
    }

    private static func read(
        _ steps: [PlanStep], profile: InjectionProfile, typed: String, undoable: Bool
    ) -> Expectation {
        var text = ""
        var caretBack = 0
        var method = InsertMethod.typed
        for step in steps {
            switch step {
            case .insertText(let inserted):
                text += inserted
                if Injector.method(for: inserted, profile: profile) == .pasted { method = .pasted }
            case .insertRich(_, let plain):
                text += plain
                method = .pasted
            case .moveCursor(let graphemes, _):
                caretBack += Int(graphemes)
            case .delete, .keyPress, .delay:
                break
            }
        }
        return Expectation(typed: typed, text: text, caretBack: caretBack, method: method, undoable: undoable)
    }
}
