import AraloBridge
import Foundation
import Observation

/// The command panel's model (PRD A3): the commands to pick from, the
/// answer as it streams in, the word diff against the selection, and what the
/// request sent.
///
/// The selection was read before the panel opened, while the app it came
/// from still had the keyboard. Nothing reaches that app until the user says
/// Replace, and then it is one paste: one Cmd+Z there brings the original back.
///
/// Every rule is the core's: which commands there are, what is sent, how an
/// answer is fitted to the selection and how the diff is cut. This holds what
/// is on screen.
@MainActor
@Observable
public final class CommandStore {
    public enum Phase: Equatable, Sendable {
        /// The list is up, and nothing has been sent.
        case choosing
        /// A command is running and the answer is streaming in.
        case running
        /// The answer is in, and the diff is there to read.
        case answered
        /// The request failed or was refused, in the core's words.
        case failed(String)
    }

    /// Every command there is: the built-in ones, then the library's.
    public let commands: [AiCommand]

    /// The commands that match the search box.
    public private(set) var rows: [AiCommand]

    /// The row Enter runs. Kept inside `rows`, and nil only when nothing
    /// matched.
    public private(set) var highlighted: String?

    /// What the search box holds.
    public var query: String = "" {
        didSet {
            guard query != oldValue else { return }
            filter()
        }
    }

    /// What was selected. Nil when there was nothing to read, and then
    /// `problem` says why.
    public let selection: String?

    /// Why no command can run, when none can.
    public let problem: String?

    public private(set) var phase: Phase = .choosing

    /// The command that ran last, or is running.
    public private(set) var running: AiCommand?

    /// The answer so far, as the model wrote it.
    public private(set) var streamed = ""

    /// What Replace puts in place of the selection: the answer fitted to it,
    /// or the user's edit of that.
    public var draft = "" {
        didSet {
            guard draft != oldValue, let selection else { return }
            diff = diffWords(before: selection, after: draft)
        }
    }

    /// The draft against the selection, word by word.
    public private(set) var diff: [DiffSpan] = []

    /// The user is editing the draft rather than reading the diff.
    public private(set) var isEditing = false

    /// What the request sent, for the chip under the answer.
    public private(set) var sent: [AiContextSent] = []

    /// The profile and model that answered.
    public private(set) var profile: String?
    public private(set) var model: String?

    /// The model stopped before it was done, at its length limit or by its
    /// own filter. The draft may be missing its end.
    public private(set) var cutShort = false

    /// Why the last Replace put nothing in, in the panel's words.
    public private(set) var refusal: String?

    @ObservationIgnored private let runner: CommandRunner
    @ObservationIgnored private let access: SelectionAccess
    @ObservationIgnored private var run: AiCommandRunProtocol?
    @ObservationIgnored private var task: Task<Void, Never>?
    /// Bumped by every run and every cancel, so an answer arriving for a run
    /// that was replaced or cancelled is dropped.
    @ObservationIgnored private var generation = 0

    public init(
        commands: [AiCommand],
        selection: Result<String, SelectionFailure>,
        runner: CommandRunner,
        access: SelectionAccess
    ) {
        self.commands = commands
        self.runner = runner
        self.access = access
        switch selection {
        case .success(let text):
            self.selection = text
            problem = nil
        case .failure(let failure):
            self.selection = nil
            problem = Self.describe(failure)
        }
        rows = commands
        highlighted = commands.first?.id
    }

    // MARK: - Choosing

    public func selectNext() {
        move(by: 1)
    }

    public func selectPrevious() {
        move(by: -1)
    }

    /// Highlights a row the user clicked. An ID that is not in the list is
    /// ignored.
    public func select(_ id: String) {
        guard rows.contains(where: { $0.id == id }) else { return }
        highlighted = id
    }

    /// Runs the highlighted command on the selection.
    public func runHighlighted() {
        guard let command = rows.first(where: { $0.id == highlighted }) else { return }
        start(command)
    }

    /// Asks the same command again, for a different answer.
    public func regenerate() {
        guard let running, phase != .running else { return }
        start(running)
    }

    /// Stops the command that is running and goes back to the list. Nothing
    /// was replaced, and nothing will be.
    public func cancel() {
        generation += 1
        run?.cancel()
        run = nil
        task = nil
        phase = .choosing
        isEditing = false
    }

    /// The panel is going away: whatever is running stops.
    public func close() {
        cancel()
    }

    /// Lets the user change the draft before it goes in.
    public func edit() {
        guard phase == .answered else { return }
        isEditing = true
    }

    // MARK: - Replacing

    /// Puts the draft in place of the selection, in the app the panel was
    /// opened over.
    ///
    /// The panel must be out of the way first: the paste goes wherever the
    /// keyboard is. True when the text went in; false means the panel should
    /// come back with `refusal` showing.
    @discardableResult
    public func replace() async -> Bool {
        guard phase == .answered, !draft.isEmpty else { return false }
        refusal = nil
        guard let failure = await access.replace(with: draft) else { return true }
        refusal = Self.describe(failure)
        return false
    }

    static func describe(_ failure: SelectionFailure) -> String {
        switch failure {
        case .refused(.paused):
            "Aralo is paused. Resume it from the menu bar, and try again."
        case .refused(.excludedApp):
            "Aralo does not read or change text in this app: everything in it is treated as a password."
        case .refused(.snippetGone):
            "That command is not in the library any more."
        case .noTargetApp:
            "Aralo could not get back to the app you came from, so it changed nothing."
        case .passwordField:
            "Aralo does not read password fields."
        case .nothingSelected:
            "Select some text first, then ask again."
        }
    }

    // MARK: - Running

    private func start(_ command: AiCommand) {
        guard let selection else { return }
        generation += 1
        let current = generation
        run?.cancel()
        run = nil
        running = command
        phase = .running
        streamed = ""
        draft = ""
        diff = []
        sent = []
        profile = nil
        model = nil
        cutShort = false
        isEditing = false
        refusal = nil
        task = Task { [runner] in
            do {
                let run = try await runner.start(command, selection: selection)
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

    private func finish(_ run: AiCommandRunProtocol) {
        self.run = nil
        cutShort = run.cutShort()
        let replacement = run.replacement()
        guard !replacement.isEmpty else {
            phase = .failed("The model answered with nothing.")
            return
        }
        draft = replacement
        diff = run.diff()
        phase = .answered
    }

    private func filter() {
        let words = query.split(whereSeparator: \.isWhitespace).map(String.init)
        rows = commands.filter { command in
            words.allSatisfy { word in
                command.label.localizedCaseInsensitiveContains(word)
                    || command.tags.contains { $0.localizedCaseInsensitiveContains(word) }
            }
        }
        if !rows.contains(where: { $0.id == highlighted }) {
            highlighted = rows.first?.id
        }
    }

    private func move(by offset: Int) {
        guard !rows.isEmpty else { return }
        let current = rows.firstIndex { $0.id == highlighted } ?? 0
        highlighted = rows[min(max(current + offset, 0), rows.count - 1)].id
    }
}

/// What runs a command: the AI settings in the app, and a fake in the tests.
public protocol CommandRunner: Sendable {
    func start(_ command: AiCommand, selection: String) async throws -> AiCommandRunProtocol
}

extension AiProfiles: CommandRunner {
    public func start(_ command: AiCommand, selection: String) async throws -> AiCommandRunProtocol {
        try await runCommand(command: command, selection: selection)
    }
}
