import AraloBridge
import Foundation
import Observation

/// Where a session's decision goes: the one place that types.
///
/// A protocol because the panel is the same whatever runs the plan. The app
/// wires in the controller that owns the injector and the queue; a test wires
/// in something that writes down what it was handed.
public protocol ExpansionRunner: AnyObject, Sendable {
    /// Runs the plan a finished session came to. Anything else is a question,
    /// and questions are the panel's business.
    func run(_ action: SessionAction)

    /// The user changed their mind: put back whatever opening the session
    /// swallowed, and forget it.
    func cancel(_ session: ExpansionSession)
}

/// One box to put in front of the user: a field of the snippet's form, in the
/// shell's own words rather than the bridge's.
public struct FormField: Identifiable, Equatable, Sendable {
    /// The name the body knows the field by, and the key its answer goes under.
    public var name: String
    /// What to write beside the box: the body's label, or the name when it gave
    /// none.
    public var label: String
    /// What is in the box before the user touches it. A drop-down's is its
    /// first choice, so a form is answerable by pressing Enter.
    public var initial: String
    /// The choices to offer, in the body's order. Empty for a box to type in.
    public var options: [String]
    /// How many lines tall the box is: one unless the body asked for room to
    /// write in, and one for a drop-down.
    public var lines: UInt32

    public var id: String { name }

    /// True for a box with room to write in, which is a box Return belongs to.
    public var hasRoomToWrite: Bool { lines > 1 }

    init(_ field: FormFieldInfo) {
        name = field.name
        label = field.label
        initial = field.default
        options = field.options
        lines = field.lines
    }
}

/// A snippet that asks something before it expands: the form panel's model.
///
/// It drives one session — the form, then whatever the body wants from outside
/// it, then the plan — and knows nothing about windows. A panel draws
/// `fields`, writes into `answers`, shows `preview`, and calls `submit` or
/// `cancel`.
///
/// Nothing reaches the document until `submit`. Asking the core a question
/// changes nothing anywhere, so a user who opens a form and thinks better of
/// it has typed nothing.
@MainActor
@Observable
public final class FormSession {
    /// The boxes to put in front of the user, in the order the body names
    /// them. Empty when the snippet asked only for something from outside,
    /// which is a session with nothing to draw.
    public private(set) var fields: [FormField] = []

    /// What is in the boxes: the defaults to begin with, then whatever the
    /// user types. A name the form does not have is ignored by the core, so a
    /// panel may leave one in here.
    public var answers: [String: String] = [:] {
        didSet {
            guard answers != oldValue else { return }
            readPreview()
        }
    }

    /// What the snippet expands to with the answers as they stand: the
    /// expansion itself, run on a copy, minus the cursor and the keys.
    public private(set) var preview: String = ""

    /// The session is over: the plan has gone to the runner, or the user
    /// cancelled, or there was never a question to ask. A panel that finds
    /// this set on opening has nothing to show.
    public private(set) var isFinished = false

    /// Which snippet is expanding.
    public var snippetId: String { session.snippetId() }

    /// What the snippet is called, for the panel's heading: a user who typed
    /// an abbreviation and got a form should see which snippet asked.
    public let label: String

    @ObservationIgnored private let session: ExpansionSession
    @ObservationIgnored private let runner: ExpansionRunner
    @ObservationIgnored private let clipboard: @MainActor () -> String?

    /// Takes over a session the engine started and asks it what it wants.
    ///
    /// `clipboard` is read only if the body asks for it: reading the
    /// pasteboard is not something to do on the chance that a snippet wanted
    /// it.
    public init(
        session: ExpansionSession,
        runner: ExpansionRunner,
        label: String = "",
        clipboard: @escaping @MainActor () -> String? = { nil }
    ) {
        self.session = session
        self.runner = runner
        self.label = label
        self.clipboard = clipboard
        advance(session.next())
    }

    /// The form is filled in: the answers go to the core, and the plan it
    /// hands back goes to the runner.
    ///
    /// The window must be out of the way before this is called. Synthetic keys
    /// follow the keyboard, so a plan run with the panel still up lands in the
    /// form.
    public func submit() {
        guard !isFinished else { return }
        advance(session.submitForm(answers: answers))
    }

    /// Never mind. What the engine swallowed to open the panel goes back into
    /// the document, and nothing else happens.
    public func cancel() {
        guard !isFinished else { return }
        isFinished = true
        runner.cancel(session)
    }

    /// Answers the question the session asked, until it has none left.
    ///
    /// The context is fetched and handed back without asking the user: a
    /// snippet that wants the clipboard is not a snippet that wants a dialog
    /// about the clipboard.
    private func advance(_ action: SessionAction) {
        switch action {
        case .form(let asked):
            fields = asked.map(FormField.init)
            // The defaults are the answers until the user changes them, so
            // the preview shows what Enter would insert from the moment the
            // panel opens.
            answers = Dictionary(fields.map { ($0.name, $0.initial) }, uniquingKeysWith: { first, _ in first })
            readPreview()
        case .context(let kinds):
            advance(session.provideContext(values: supply(for: kinds)))
        case .expand:
            isFinished = true
            runner.run(action)
        case .done:
            isFinished = true
        }
    }

    /// What the shell can fetch, of what was asked for.
    ///
    /// Only the clipboard so far. The selection, the app and the window are
    /// left unsupplied, and the core leaves those placeholders as written
    /// rather than expanding them to nothing.
    ///
    /// A pasteboard with no text on it is an empty clipboard, not a missing
    /// one: it was read, and what it holds as text is nothing. Copying an
    /// image must not leave `{{clipboard}}` sitting in the document.
    private func supply(for kinds: [ContextNeed]) -> ContextSupply {
        ContextSupply(
            clipboard: kinds.contains(.clipboard) ? clipboard() ?? "" : nil,
            selection: nil,
            app: nil,
            window: nil
        )
    }

    private func readPreview() {
        preview = session.previewWith(answers: answers)
    }
}
