import AraloBridge
import Foundation

// What a session exchanges with the shell around it: where its plan goes, the
// boxes of its form, and the context the shell can fetch for it.

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

/// What the shell can fetch for a session. Each is read only when the snippet
/// asks for it: the clipboard by its body, the rest by its AI blocks'
/// declarations. A kind the shell cannot fetch answers nil, and the core then
/// sends nothing for it.
public struct SessionContext {
    public var clipboard: @MainActor () -> String?
    public var selection: @MainActor () -> String?
    public var app: @MainActor () -> String?
    public var window: @MainActor () -> String?

    public init(
        clipboard: @escaping @MainActor () -> String? = { nil },
        selection: @escaping @MainActor () -> String? = { nil },
        app: @escaping @MainActor () -> String? = { nil },
        window: @escaping @MainActor () -> String? = { nil }
    ) {
        self.clipboard = clipboard
        self.selection = selection
        self.app = app
        self.window = window
    }
}
