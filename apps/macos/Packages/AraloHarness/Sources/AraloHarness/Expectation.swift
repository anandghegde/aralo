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
        case needsContext(String)
        /// The body has an AI block: what goes in is a model's to write, and
        /// there is no expectation to hold the app to.
        case needsModel(String)

        public var description: String {
            switch self {
            case .noExpansion(let abbreviation):
                "the library does not expand \"\(abbreviation)\"; is it fixtures/matrix/library?"
            case .needsContext(let abbreviation):
                "\"\(abbreviation)\" wants something from outside the core that the harness cannot supply"
            case .needsModel(let abbreviation):
                "\"\(abbreviation)\" asks a model for part of its text, which the harness cannot predict"
            }
        }
    }

    public static let delimiter = " "

    /// What the harness types into every box of a form panel. One word of
    /// lower-case letters, so any Latin layout can type it, and nothing a
    /// matrix body says itself, so finding it in the field means the answer
    /// went in.
    public static let formAnswer = "dana"

    /// Types the abbreviation into `engine` and reads the plan that comes back.
    public static func ask(_ engine: Engine, abbreviation: String) throws -> Expectation {
        engine.reset(reason: .manual)
        let typed = abbreviation + delimiter
        var answer: Expectation?
        for scalar in typed.unicodeScalars {
            switch engine.onKey(key: .char(scalar: scalar.value)) {
            case .expand(_, _, let steps, let undoDeleteCount, let profile):
                answer = read(steps, profile: profile, typed: typed, undoable: undoDeleteCount != nil)
            case .startSession(_, _, let session):
                answer = try drive(session, typed: typed, abbreviation: abbreviation)
            case .pass, .undoExpansion:
                continue
            }
        }
        engine.reset(reason: .manual)
        guard let answer else { throw Problem.noExpansion(abbreviation) }
        return answer
    }

    /// A body that asks something is driven the way the panel drives it, with
    /// `formAnswer` in every box: what the harness is about to type, so what
    /// comes back is what the app should end up with.
    ///
    /// Asking the core changes nothing, so this costs the matrix nothing but the
    /// call. The phases are the session's own — the form, then what the body
    /// wants from outside it — so the walk is bounded by them.
    private static func drive(
        _ session: ExpansionSession, typed: String, abbreviation: String
    ) throws -> Expectation? {
        var action = session.next()
        for _ in 0...sessionQuestions {
            switch action {
            case .form(let fields):
                let answers = fields.map { ($0.name, formAnswer) }
                action = session.submitForm(answers: Dictionary(answers, uniquingKeysWith: { first, _ in first }))
            case .context:
                // The clipboard and the rest are the shell's to fetch, and the
                // harness is not the shell. A matrix body that wants them would
                // be compared against a guess, so say so instead.
                throw Problem.needsContext(abbreviation)
            case .ai:
                throw Problem.needsModel(abbreviation)
            case .expand(_, let steps, let undoDeleteCount, let profile):
                return read(steps, profile: profile, typed: typed, undoable: undoDeleteCount != nil)
            case .done:
                return nil
            }
        }
        return nil
    }

    /// How many questions a session asks before it has a plan: the form, then
    /// whatever the body wants from outside it. A bound, so a session that keeps
    /// asking cannot keep the harness in a loop.
    private static let sessionQuestions = 2

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
