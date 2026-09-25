import AppKit
import AraloBridge
import Foundation
import Observation

/// The editor's AI sheet (PRD A1): one action on the selected part of the
/// body, or on all of it, and the answer as it streams in, against what it
/// would replace.
///
/// Nothing in the editor changes until the user says Replace, and then it is
/// one edit: one undo in the editor brings the text back. The core decides
/// what is sent, how an answer is fitted to the text it replaces, and which
/// placeholders it dropped or added; this holds what is on screen.
@MainActor
@Observable
public final class AuthoringStore {
    public enum Phase: Equatable, Sendable {
        /// A draft, waiting for the user to say what the snippet is for.
        case asking
        /// The answer is streaming in.
        case running
        /// The answer is in, to read, choose from, edit and put in.
        case answered
        /// The request failed or was refused, in the core's words.
        case failed(String)
    }

    /// What the menu called the action, in the core's words.
    public private(set) var label: String

    /// The text the answer replaces: the selection, or the whole body.
    public let replacing: String

    /// Where that text is in the body, in UTF-16 code units.
    public let range: NSRange

    public private(set) var phase: Phase

    /// For a draft: what the snippet should say, beyond its label.
    public var note = ""

    /// The answer so far, as the model wrote it.
    public private(set) var streamed = ""

    /// The versions the answer holds: several for variations, one otherwise.
    public private(set) var versions: [String] = []

    /// Which version `draft` started from.
    public private(set) var chosen = 0

    /// What Replace puts in: the chosen version, or the user's edit of it.
    public var draft = "" {
        didSet {
            guard draft != oldValue else { return }
            recheck()
        }
    }

    /// The draft against the text it replaces, word by word.
    public private(set) var diff: [DiffSpan] = []

    /// Placeholders the draft dropped or added. A snippet body is a template,
    /// and a placeholder a model wrote is one the snippet will expand.
    public private(set) var changes: [AiPlaceholderChange] = []

    /// The user is editing the draft rather than reading the diff.
    public private(set) var isEditing = false

    /// What the request sent, and who answered.
    public private(set) var sent: [AiContextSent] = []
    public private(set) var profile: String?
    public private(set) var model: String?

    /// The model stopped before it was done. The draft may be missing its end.
    public private(set) var cutShort = false

    @ObservationIgnored private var action: AiAuthoring
    @ObservationIgnored private let snippetLabel: String
    @ObservationIgnored private let runner: AuthoringRunner
    @ObservationIgnored private var run: AiAuthoringRunProtocol?
    @ObservationIgnored private var task: Task<Void, Never>?
    /// Bumped by every run and every cancel, so an answer arriving for a run
    /// that was replaced or cancelled is dropped.
    @ObservationIgnored private var generation = 0

    /// Opens the sheet for `action` on `replacing`, which is `range` of the
    /// body. Every action but a draft starts at once; a draft first asks what
    /// the snippet is for.
    public init(action: AiAuthoring, replacing: String, range: NSRange, runner: AuthoringRunner) {
        self.action = action
        self.replacing = replacing
        self.range = range
        self.runner = runner
        label = authoringLabel(action: action)
        if case .draft(let named, _) = action {
            snippetLabel = named
            phase = .asking
        } else {
            snippetLabel = ""
            phase = .running
            start()
        }
    }

    /// Whether the action is a draft, which sends the label and a note rather
    /// than any of the text.
    public var isDraft: Bool {
        if case .draft = action { return true }
        return false
    }

    /// Whether Replace can put the draft in now.
    public var canReplace: Bool {
        phase == .answered && !draft.isEmpty
    }

    /// Whether a draft has something to go on: the snippet's label, or a
    /// note.
    public var canDraft: Bool {
        !snippetLabel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !note.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Drafts from the label and the note the user wrote.
    public func draftNow() {
        guard case .draft = action, phase != .running else { return }
        action = .draft(label: snippetLabel, note: note)
        start()
    }

    /// Asks again, for a different answer.
    public func regenerate() {
        guard phase != .running, phase != .asking else { return }
        if case .draft = action {
            action = .draft(label: snippetLabel, note: note)
        }
        start()
    }

    /// Starts from another version.
    public func choose(_ index: Int) {
        guard versions.indices.contains(index) else { return }
        chosen = index
        draft = versions[index]
        isEditing = false
    }

    /// Lets the user change the draft before it goes in.
    public func edit() {
        guard phase == .answered else { return }
        isEditing = true
    }

    /// Stops the answer that is coming. Nothing was put in, and nothing will
    /// be.
    public func cancel() {
        generation += 1
        run?.cancel()
        run = nil
        task = nil
        if phase == .running {
            phase = versions.isEmpty ? .failed("You stopped it before the model answered.") : .answered
        }
    }

    /// The sheet is going away: whatever is running stops.
    public func close() {
        cancel()
    }

    private func start() {
        generation += 1
        let current = generation
        run?.cancel()
        run = nil
        phase = .running
        streamed = ""
        versions = []
        chosen = 0
        draft = ""
        diff = []
        changes = []
        sent = []
        profile = nil
        model = nil
        cutShort = false
        isEditing = false
        let asked = self.action
        let text = replacing
        task = Task { [runner] in
            do {
                let run = try await runner.start(asked, text: text)
                guard current == generation else {
                    run.cancel()
                    return
                }
                self.run = run
                profile = run.profile()
                model = run.model()
                sent = run.sent()
                while let piece = try await run.next() {
                    guard current == generation else { return }
                    streamed += piece
                }
                guard current == generation else { return }
                finish(run)
            } catch {
                guard current == generation else { return }
                phase = .failed(error.reason)
            }
        }
    }

    private func finish(_ run: AiAuthoringRunProtocol) {
        self.run = nil
        cutShort = run.cutShort()
        versions = run.versions()
        guard let first = versions.first else {
            phase = .failed("The model answered with nothing.")
            return
        }
        draft = first
        phase = .answered
    }

    private func recheck() {
        diff = diffWords(before: replacing, after: draft)
        changes = placeholderChanges(before: replacing, after: draft)
    }
}

/// What runs an editor action: the AI settings in the app, and a fake in the
/// tests.
public protocol AuthoringRunner: Sendable {
    func start(_ action: AiAuthoring, text: String) async throws -> AiAuthoringRunProtocol
}

extension AiProfiles: AuthoringRunner {
    public func start(_ action: AiAuthoring, text: String) async throws -> AiAuthoringRunProtocol {
        try await runAuthoring(action: action, text: text)
    }
}

/// Puts an answer into a text view as one edit.
@MainActor
public enum TextReplacement {
    /// Replaces `range` of the view's text with `text`, as one step of the
    /// view's own undo named `action`, and selects what went in. False, and
    /// nothing changed, when the range no longer holds `expected`: the text
    /// moved on while the answer was being read.
    @discardableResult
    public static func replace(
        in view: NSTextView, range: NSRange, expected: String, with text: String, action: String
    ) -> Bool {
        let current = view.string as NSString
        guard NSMaxRange(range) <= current.length, current.substring(with: range) == expected else {
            return false
        }
        let undo = view.undoManager
        view.breakUndoCoalescing()
        undo?.beginUndoGrouping()
        view.insertText(text, replacementRange: range)
        undo?.setActionName(action)
        undo?.endUndoGrouping()
        view.breakUndoCoalescing()
        view.setSelectedRange(NSRange(location: range.location, length: (text as NSString).length))
        view.scrollRangeToVisible(view.selectedRange())
        return true
    }
}
