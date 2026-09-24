import AraloBridge
@testable import AraloKit
import XCTest

/// The command panel's model (task 4.5), with the run and the app faked.
@MainActor
final class CommandStoreTests: XCTestCase {
    private let proofread = AiCommand(
        id: "01K5C0MMANDS00000000000001", label: "Fix spelling and grammar",
        instruction: "Fix it.", tags: ["writing"], profile: nil, model: nil, builtin: true
    )
    private let shorter = AiCommand(
        id: "01K5C0MMANDS00000000000003", label: "Make it shorter",
        instruction: "Shorter.", tags: [], profile: nil, model: nil, builtin: true
    )

    private func store(
        selection: Result<String, SelectionFailure> = .success("their going home"),
        runner: FakeRunner,
        access: FakeAccess = FakeAccess()
    ) -> CommandStore {
        CommandStore(commands: [proofread, shorter], selection: selection, runner: runner, access: access)
    }

    func testARunStreamsTheAnswerAndEndsWithADiff() async throws {
        let runner = FakeRunner(answer: "they're going home")
        let store = store(runner: runner)
        store.runHighlighted()
        XCTAssertEqual(store.phase, .running)
        try await until { store.phase == .answered }

        XCTAssertEqual(runner.asked.map(\.command.id), [proofread.id])
        XCTAssertEqual(runner.asked.first?.selection, "their going home")
        XCTAssertEqual(store.streamed, "they're going home")
        XCTAssertEqual(store.draft, "they're going home")
        XCTAssertEqual(store.profile, "Example")
        XCTAssertEqual(store.model, "model-a")
        XCTAssertEqual(store.sent, [AiContextSent(kind: .selection, bytes: 16)])
        XCTAssertEqual(store.diff.first, DiffSpan(change: .removed, text: "their"))
        XCTAssertEqual(store.diff.dropFirst().first, DiffSpan(change: .added, text: "they're"))
    }

    func testTheSearchBoxNarrowsTheListByLabelAndTag() {
        let store = store(runner: FakeRunner(answer: ""))
        store.query = "short"
        XCTAssertEqual(store.rows.map(\.id), [shorter.id])
        XCTAssertEqual(store.highlighted, shorter.id)
        store.query = "writing"
        XCTAssertEqual(store.rows.map(\.id), [proofread.id])
        store.query = ""
        store.selectNext()
        store.selectNext()
        XCTAssertEqual(store.highlighted, shorter.id, "clamped at the bottom")
    }

    func testWithNothingSelectedNothingRunsAndThePanelSaysWhy() {
        let runner = FakeRunner(answer: "x")
        let store = store(selection: .failure(.nothingSelected), runner: runner)
        XCTAssertNil(store.selection)
        XCTAssertEqual(store.problem, "Select some text first, then ask again.")
        store.runHighlighted()
        XCTAssertEqual(store.phase, .choosing)
        XCTAssertTrue(runner.asked.isEmpty)
    }

    func testARefusalIsShownInTheCoresWords() async throws {
        let runner = FakeRunner(answer: "x")
        runner.error = AiBridgeError.Refused(message: "AI is off. Turn it on in Settings.")
        let store = store(runner: runner)
        store.runHighlighted()
        try await until { store.phase != .running }
        XCTAssertEqual(store.phase, .failed("AI is off. Turn it on in Settings."))
    }

    func testCancelStopsTheRunAndALateAnswerIsDropped() async throws {
        let runner = FakeRunner(answer: "never shown", hang: true)
        let store = store(runner: runner)
        store.runHighlighted()
        try await until { runner.runs.first?.isWaiting == true }
        store.cancel()
        XCTAssertEqual(store.phase, .choosing)
        XCTAssertTrue(runner.runs[0].cancelled)
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertEqual(store.phase, .choosing)
        XCTAssertEqual(store.draft, "")
    }

    func testRegenerateAsksAgainAndEditingChangesWhatIsReplaced() async throws {
        let runner = FakeRunner(answer: "they're going home")
        let access = FakeAccess()
        let store = store(runner: runner, access: access)
        store.runHighlighted()
        try await until { store.phase == .answered }
        store.regenerate()
        try await until { store.phase == .answered }
        XCTAssertEqual(runner.asked.count, 2)

        store.edit()
        XCTAssertTrue(store.isEditing)
        store.draft = "They are going home"
        XCTAssertTrue(store.diff.contains(DiffSpan(change: .added, text: "They are")))
        let replaced = await store.replace()
        XCTAssertTrue(replaced)
        XCTAssertEqual(access.replaced, ["They are going home"])
    }

    func testAReplaceThatCannotGoInKeepsThePanelWithTheReason() async throws {
        let access = FakeAccess()
        access.failure = .refused(.paused)
        let store = store(runner: FakeRunner(answer: "they're going home"), access: access)
        let early = await store.replace()
        XCTAssertFalse(early, "nothing to replace before an answer")
        store.runHighlighted()
        try await until { store.phase == .answered }
        let replaced = await store.replace()
        XCTAssertFalse(replaced)
        XCTAssertEqual(store.refusal, "Aralo is paused. Resume it from the menu bar, and try again.")
    }

    /// Waits for the run's task to get somewhere, for at most a second.
    private func until(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(1)
        while !condition() {
            guard ContinuousClock.now < deadline else { return XCTFail("timed out") }
            try await Task.sleep(for: .milliseconds(2))
        }
    }
}

/// Answers every command with `answer` in two pieces, or throws `error`.
private final class FakeRunner: CommandRunner, @unchecked Sendable {
    let answer: String
    let hang: Bool
    var error: Error?
    private(set) var asked: [(command: AiCommand, selection: String)] = []
    private(set) var runs: [FakeRun] = []

    init(answer: String, hang: Bool = false) {
        self.answer = answer
        self.hang = hang
    }

    func start(_ command: AiCommand, selection: String) async throws -> AiCommandRunProtocol {
        try await MainActor.run {
            asked.append((command, selection))
            if let error { throw error }
            let run = FakeRun(selection: selection, answer: answer, hang: hang)
            runs.append(run)
            return run
        }
    }
}

private final class FakeRun: AiCommandRunProtocol, @unchecked Sendable {
    private let lock = NSLock()
    private let selection: String
    private let answer: String
    private let hang: Bool
    private var pieces: [String]
    private var waiting: CheckedContinuation<String?, Never>?
    private var wasCancelled = false

    init(selection: String, answer: String, hang: Bool) {
        self.selection = selection
        self.answer = answer
        self.hang = hang
        let middle = answer.index(answer.startIndex, offsetBy: answer.count / 2)
        pieces = hang ? [] : [String(answer[..<middle]), String(answer[middle...])]
    }

    var cancelled: Bool { lock.withLock { wasCancelled } }
    var isWaiting: Bool { lock.withLock { waiting != nil } }

    func next() async throws -> String? {
        if let piece = lock.withLock({ pieces.isEmpty ? nil : pieces.removeFirst() }) {
            return piece
        }
        guard hang else { return nil }
        return await withCheckedContinuation { continuation in
            lock.withLock {
                if wasCancelled {
                    continuation.resume(returning: nil)
                } else {
                    waiting = continuation
                }
            }
        }
    }

    func cancel() {
        lock.withLock {
            wasCancelled = true
            waiting?.resume(returning: nil)
            waiting = nil
        }
    }

    func cutShort() -> Bool { false }
    func diff() -> [DiffSpan] { diffWords(before: selection, after: answer) }
    func model() -> String { "model-a" }
    func profile() -> String { "Example" }
    func replacement() -> String { answer }
    func sent() -> [AiContextSent] { [AiContextSent(kind: .selection, bytes: UInt64(selection.utf8.count))] }
    func text() -> String { answer }
}

@MainActor
private final class FakeAccess: SelectionAccess {
    var failure: SelectionFailure?
    private(set) var replaced: [String] = []

    func read() async -> Result<String, SelectionFailure> { .success("their going home") }

    func replace(with text: String) async -> SelectionFailure? {
        if failure == nil { replaced.append(text) }
        return failure
    }
}
