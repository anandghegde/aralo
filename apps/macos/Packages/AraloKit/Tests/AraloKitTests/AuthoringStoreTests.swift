import AppKit
import AraloBridge
@testable import AraloKit
import XCTest

/// The editor's AI sheet (task 4.7), with the run faked, and the edit that puts
/// an answer in, against a real text view and its undo.
@MainActor
final class AuthoringStoreTests: XCTestCase {
    private func store(
        _ action: AiAuthoring, replacing: String, runner: FakeAuthoringRunner
    ) -> AuthoringStore {
        AuthoringStore(
            action: action, replacing: replacing,
            range: NSRange(location: 0, length: (replacing as NSString).length), runner: runner
        )
    }

    func testAnAnswerStreamsInAndIsReadAgainstWhatItReplaces() async throws {
        let runner = FakeAuthoringRunner(versions: ["Hello {{clipboard}}, see you soon!"])
        let store = store(.friendlier, replacing: "Hi {{field: who}}, bye.", runner: runner)
        XCTAssertEqual(store.phase, .running)
        XCTAssertEqual(store.label, "Make it friendlier")
        try await until { store.phase == .answered }

        XCTAssertEqual(runner.asked.map(\.text), ["Hi {{field: who}}, bye."])
        XCTAssertEqual(store.streamed, "Hello {{clipboard}}, see you soon!")
        XCTAssertEqual(store.draft, "Hello {{clipboard}}, see you soon!")
        XCTAssertEqual(store.profile, "Example")
        XCTAssertEqual(store.sent, [AiContextSent(kind: .selection, bytes: 23)])
        XCTAssertTrue(store.canReplace)
        XCTAssertFalse(store.diff.isEmpty)
        // A snippet body is a template: what the answer did to its
        // placeholders is said before it goes in.
        XCTAssertEqual(store.changes, [
            AiPlaceholderChange(change: .removed, placeholder: "{{field: who}}"),
            AiPlaceholderChange(change: .added, placeholder: "{{clipboard}}")
        ])
    }

    func testAVersionCanBeChosenAndEditedBeforeItGoesIn() async throws {
        let runner = FakeAuthoringRunner(versions: ["Thanks!", "Thank you so much!", "Cheers!"])
        let store = store(.variations, replacing: "Thanks", runner: runner)
        try await until { store.phase == .answered }
        XCTAssertEqual(store.versions.count, 3)
        XCTAssertEqual(store.draft, "Thanks!")

        store.choose(2)
        XCTAssertEqual(store.chosen, 2)
        XCTAssertEqual(store.draft, "Cheers!")
        store.edit()
        XCTAssertTrue(store.isEditing)
        store.draft = "Cheers, {{field: who}}!"
        XCTAssertEqual(store.changes, [AiPlaceholderChange(change: .added, placeholder: "{{field: who}}")])

        store.regenerate()
        try await until { store.phase == .answered }
        XCTAssertEqual(runner.asked.count, 2)
        XCTAssertEqual(store.draft, "Thanks!", "a new answer starts again from its first version")
    }

    func testADraftWaitsForSomethingToGoOn() async throws {
        let runner = FakeAuthoringRunner(versions: ["Thanks for your order, {{field: who}}!"])
        let store = store(.draft(label: "", note: ""), replacing: "", runner: runner)
        XCTAssertEqual(store.phase, .asking)
        XCTAssertFalse(store.canDraft)
        XCTAssertTrue(runner.asked.isEmpty, "nothing is sent until the user says what it is for")

        store.note = "A thank-you for an order"
        XCTAssertTrue(store.canDraft)
        store.draftNow()
        try await until { store.phase == .answered }
        XCTAssertEqual(runner.asked.map(\.action), [.draft(label: "", note: "A thank-you for an order")])
        XCTAssertEqual(store.draft, "Thanks for your order, {{field: who}}!")
    }

    func testARefusalIsShownInTheCoresWords() async throws {
        let runner = FakeAuthoringRunner(versions: ["unused"])
        runner.error = AiBridgeError.Refused(message: "AI is switched off")
        let store = store(.proofread, replacing: "their going", runner: runner)
        try await until { store.phase != .running }
        XCTAssertEqual(store.phase, .failed("AI is switched off"))
        XCTAssertFalse(store.canReplace)
    }

    func testCancelStopsTheAnswerAndALateOneIsDropped() async throws {
        let runner = FakeAuthoringRunner(versions: [], hang: true)
        let store = store(.shorter, replacing: "A long text", runner: runner)
        try await until { runner.runs.first?.isWaiting == true }
        store.cancel()
        XCTAssertTrue(runner.runs[0].cancelled)
        XCTAssertEqual(store.phase, .failed("You stopped it before the model answered."))
        XCTAssertFalse(store.canReplace)
    }

    // MARK: - Putting it in

    func testAnAnswerGoesInAsOneEditTheViewsUndoTakesBack() {
        let undo = UndoSource()
        let view = NSTextView(frame: NSRect(x: 0, y: 0, width: 300, height: 100))
        view.allowsUndo = true
        view.delegate = undo
        view.string = "Dear Sam, their going home."
        let range = NSRange(location: 10, length: 17)

        XCTAssertTrue(TextReplacement.replace(
            in: view, range: range, expected: "their going home.", with: "they're going home.",
            action: "Fix spelling and grammar"
        ))
        XCTAssertEqual(view.string, "Dear Sam, they're going home.")
        XCTAssertEqual(view.selectedRange(), NSRange(location: 10, length: 19))
        XCTAssertEqual(undo.manager.undoActionName, "Fix spelling and grammar")

        undo.manager.undo()
        XCTAssertEqual(view.string, "Dear Sam, their going home.")
    }

    func testTextThatMovedOnUnderTheSheetIsLeftAlone() {
        let view = NSTextView(frame: NSRect(x: 0, y: 0, width: 300, height: 100))
        view.string = "Something else now"
        XCTAssertFalse(TextReplacement.replace(
            in: view, range: NSRange(location: 0, length: 5), expected: "Their", with: "They're",
            action: "Fix spelling and grammar"
        ))
        XCTAssertFalse(TextReplacement.replace(
            in: view, range: NSRange(location: 10, length: 50), expected: "", with: "x", action: "x"
        ))
        XCTAssertEqual(view.string, "Something else now")
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

/// Gives a text view with no window an undo manager of its own. It groups
/// only when asked, since a test has no run loop to close a group for it.
@MainActor
private final class UndoSource: NSObject, NSTextViewDelegate {
    let manager: UndoManager = {
        let manager = UndoManager()
        manager.groupsByEvent = false
        return manager
    }()

    func undoManager(for view: NSTextView) -> UndoManager? {
        manager
    }
}

/// Answers every action with `versions`, all of it in one piece, or throws
/// `error`, or waits until it is cancelled.
private final class FakeAuthoringRunner: AuthoringRunner, @unchecked Sendable {
    let versions: [String]
    let hang: Bool
    var error: Error?
    private(set) var asked: [(action: AiAuthoring, text: String)] = []
    private(set) var runs: [FakeAuthoringRun] = []

    init(versions: [String], hang: Bool = false) {
        self.versions = versions
        self.hang = hang
    }

    func start(_ action: AiAuthoring, text: String) async throws -> AiAuthoringRunProtocol {
        try await MainActor.run {
            asked.append((action, text))
            if let error { throw error }
            let run = FakeAuthoringRun(original: text, versions: versions, hang: hang)
            runs.append(run)
            return run
        }
    }
}

private final class FakeAuthoringRun: AiAuthoringRunProtocol, @unchecked Sendable {
    private let lock = NSLock()
    private let originalText: String
    private let answers: [String]
    private let hang: Bool
    private var pending: [String]
    private var waiting: CheckedContinuation<String?, Never>?
    private var wasCancelled = false

    init(original: String, versions: [String], hang: Bool) {
        originalText = original
        answers = versions
        self.hang = hang
        pending = hang ? [] : [versions.joined(separator: "\n%%%\n")]
    }

    var cancelled: Bool { lock.withLock { wasCancelled } }
    var isWaiting: Bool { lock.withLock { waiting != nil } }

    func next() async throws -> String? {
        let piece = lock.withLock { () -> String? in
            pending.isEmpty ? nil : pending.removeFirst()
        }
        if let piece {
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
    func model() -> String { "model-a" }
    func original() -> String { originalText }
    func profile() -> String { "Example" }
    func sent() -> [AiContextSent] { [AiContextSent(kind: .selection, bytes: UInt64(originalText.utf8.count))] }
    func text() -> String { answers.joined(separator: "\n%%%\n") }
    func versions() -> [String] { hang ? [] : answers }
}
