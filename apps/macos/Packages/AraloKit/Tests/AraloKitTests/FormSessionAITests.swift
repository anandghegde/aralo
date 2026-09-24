import AraloBridge
@testable import AraloKit
import XCTest

/// The form panel's AI step (task 4.6), against the real core with the model
/// faked: a snippet's `{{ai}}` blocks stream into the preview, and Enter puts
/// in what is there. With no one to ask, the fallback goes in and the panel
/// says why.
@MainActor
final class FormSessionAITests: XCTestCase {
    private var folder: URL!
    private var cache: URL!
    private var core: Core!
    private var runner: RecordingRunner!

    private enum Problem: Error {
        /// The engine expanded instead of asking, which is the test's fault.
        case nothingWasAsked
    }

    override func setUpWithError() throws {
        let run = UUID().uuidString
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)")
        cache = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)-cache")
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil, trash: nil)
        core.engine().setFrontApp(bundleId: "com.apple.TextEdit")
        runner = RecordingRunner()
    }

    override func tearDownWithError() throws {
        runner = nil
        core = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    /// A snippet picked from the palette, with `models` to answer its blocks.
    private func picked(_ body: String, models: BlockRunner? = nil) throws -> FormSession {
        let id = try core.createSnippet(
            group: [],
            draft: SnippetDraft(
                label: "Reply", abbreviations: [";rp"], body: body, tags: [], kind: .text,
                trigger: nil, case: nil, wholeWord: nil, keepDelimiter: nil, enabled: nil
            )
        )
        try core.reload()
        return try session(for: id, models: models)
    }

    private func session(
        for id: String, context: SessionContext = SessionContext(), models: BlockRunner?
    ) throws -> FormSession {
        guard case .startSession(_, let session) = core.engine()
            .insert(snippetId: id, intoApp: "com.apple.TextEdit")
        else {
            throw Problem.nothingWasAsked
        }
        return FormSession(session: session, runner: runner, label: "Reply", context: context, models: models)
    }

    func testAFormThenABlockStreamsIntoThePreviewAndEnterPutsItIn() async throws {
        let models = FakeBlockRunner(answer: "Thank you!")
        let form = try picked("Hi {{field: who}},\n{{ai: Thank them | fallback: Thanks.}}", models: models)
        XCTAssertEqual(form.step, .form)
        // The form's Enter moves on to the block rather than typing.
        XCTAssertFalse(form.insertsOnSubmit)
        XCTAssertEqual(form.preview, "Hi ,\nThanks.")

        form.answers["who"] = "Dana"
        form.submit()
        XCTAssertEqual(form.step, .blocks)
        XCTAssertFalse(form.isFinished)
        XCTAssertEqual(form.blocks.map(\.prompt), ["Thank them"])
        try await until { !form.isWriting }

        let block = try XCTUnwrap(form.blocks.first)
        XCTAssertEqual(block.status, .written)
        XCTAssertEqual(block.text, "Thank you!")
        XCTAssertEqual(block.model, "model-a")
        XCTAssertEqual(block.sent, [AiContextSent(kind: .fillins, bytes: 9)])
        XCTAssertEqual(form.preview, "Hi Dana,\nThank you!")
        // The model's text is marked; the rest is the snippet's own.
        XCTAssertEqual(form.marked, [BlockSpan(block: 0, start: 9, length: 10)])
        XCTAssertEqual(models.asked, [0])
        XCTAssertTrue(runner.actions.isEmpty, "nothing is typed while the answer is read")

        XCTAssertTrue(form.insertsOnSubmit)
        form.submit()
        XCTAssertTrue(form.isFinished)
        XCTAssertEqual(insertedText(runner.actions.first), "Hi Dana,\nThank you!")
    }

    func testWithNoOneToAskTheFallbackGoesInAndThePanelSaysWhy() async throws {
        let form = try picked("{{ai: Thank them | fallback: Thanks.}}")
        // No form, so the panel opens on the block.
        XCTAssertEqual(form.step, .blocks)
        XCTAssertFalse(form.isFinished)
        try await until { !form.isWriting }
        XCTAssertEqual(form.blocks.first?.status, .failed("AI is not set up here"))
        XCTAssertEqual(form.preview, "Thanks.")

        form.submit()
        XCTAssertEqual(insertedText(runner.actions.first), "Thanks.")
    }

    func testARefusalIsTheReasonTheFallbackGoesIn() async throws {
        let models = FakeBlockRunner(answer: "never")
        models.error = AiBridgeError.Refused(message: "AI is switched off")
        let form = try picked("<{{ai: Anything | fallback: offline}}>", models: models)
        try await until { !form.isWriting }
        XCTAssertEqual(form.blocks.first?.status, .failed("AI is switched off"))
        XCTAssertEqual(form.preview, "<offline>")
        form.submit()
        XCTAssertEqual(insertedText(runner.actions.first), "<offline>")
    }

    func testRegenerateAsksAgainAndAnEditIsWhatGoesIn() async throws {
        let models = FakeBlockRunner(answer: "Thanks a lot")
        let form = try picked("[{{ai: Thank them}}]", models: models)
        try await until { !form.isWriting }
        form.regenerate()
        try await until { !form.isWriting }
        XCTAssertEqual(models.asked, [0, 0])

        form.edit()
        XCTAssertEqual(form.editing, 0)
        form.setText("My own words", for: 0)
        XCTAssertEqual(form.preview, "[My own words]")
        form.submit()
        XCTAssertEqual(insertedText(runner.actions.first), "[My own words]")
    }

    func testTheFallbackCanBeChosenOverTheAnswer() async throws {
        let form = try picked("{{ai: Thank them | fallback: Thanks.}}", models: FakeBlockRunner(answer: "Cheers"))
        try await until { !form.isWriting }
        form.useFallback(0)
        XCTAssertEqual(form.preview, "Thanks.")
        form.submit()
        XCTAssertEqual(insertedText(runner.actions.first), "Thanks.")
    }

    func testNothingGoesInWhileAModelIsWritingAndCancelStopsIt() async throws {
        let models = FakeBlockRunner(answer: "", hang: true)
        let form = try picked("{{ai: Thank them | fallback: Thanks.}}", models: models)
        try await until { models.runs.first?.isWaiting == true }
        XCTAssertTrue(form.isWriting)
        XCTAssertFalse(form.canSubmit)
        form.submit()
        XCTAssertTrue(runner.actions.isEmpty)

        form.cancel()
        XCTAssertTrue(form.isFinished)
        XCTAssertTrue(models.runs[0].cancelled)
        XCTAssertEqual(runner.cancelled, 1)
        XCTAssertTrue(runner.actions.isEmpty)
    }

    func testStopKeepsWhatArrivedToReadAndEdit() async throws {
        let models = FakeBlockRunner(answer: "", hang: true, first: "The first half")
        let form = try picked("{{ai: Thank them | fallback: Thanks.}}", models: models)
        try await until { models.runs.first?.isWaiting == true }
        XCTAssertEqual(form.preview, "The first half")

        form.stop()
        let block = try XCTUnwrap(form.blocks.first)
        XCTAssertEqual(block.status, .written)
        XCTAssertTrue(block.cutShort)
        XCTAssertTrue(models.runs[0].cancelled)
        form.submit()
        XCTAssertEqual(insertedText(runner.actions.first), "The first half")
    }

    /// The context a snippet declared for its blocks is read, and nothing
    /// else is: not the clipboard, not the selection.
    func testOnlyTheDeclaredContextIsRead() async throws {
        let file = "---\nlabel: Declared\nai:\n  context: [app, window]\n---\n{{ai: Reply to them}}\n"
        try file.write(to: folder.appendingPathComponent("declared.md"), atomically: true, encoding: .utf8)
        try core.reload()
        let id = try XCTUnwrap(core.snippets().first { $0.label == "Declared" }?.id)
        let read = ReadLog()
        let form = try session(
            for: id,
            context: SessionContext(
                clipboard: { read.note("clipboard", "copied") },
                selection: { read.note("selection", "selected") },
                app: { read.note("app", "Mail") },
                window: { read.note("window", "Inbox") }
            ),
            models: FakeBlockRunner(answer: "Hello")
        )
        XCTAssertEqual(read.kinds, ["app", "window"])
        try await until { !form.isWriting }
        form.submit()
        XCTAssertEqual(insertedText(runner.actions.first), "Hello")
    }

    func testMarkedRangesFollowUTF16AndSkipWhatIsNotThere() {
        let text = "👋 Hi, Dana!"
        let ranges = markedRanges(in: text, spans: [
            BlockSpan(block: 0, start: 3, length: 3),
            BlockSpan(block: 1, start: 40, length: 2)
        ])
        XCTAssertEqual(ranges.map { String(text[$0]) }, ["Hi,"])
    }

    /// Waits for the model's task to get somewhere, for at most a second.
    private func until(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(1)
        while !condition() {
            guard ContinuousClock.now < deadline else { return XCTFail("timed out") }
            try await Task.sleep(for: .milliseconds(2))
        }
    }

    /// The text an `.expand` action would insert.
    private func insertedText(_ action: SessionAction?) -> String {
        guard case .expand(_, let steps, _, _) = action else { return "" }
        return steps.reduce(into: "") { text, step in
            if case .insertText(let inserted) = step { text += inserted }
        }
    }
}

/// Which kinds of context a session read, in order.
@MainActor
private final class ReadLog {
    private(set) var kinds: [String] = []

    func note(_ kind: String, _ value: String) -> String? {
        kinds.append(kind)
        return value
    }
}

/// Answers every block with `answer` in two pieces, or throws `error`. A run
/// that hangs sends `first`, if anything, then waits until it is cancelled.
private final class FakeBlockRunner: BlockRunner, @unchecked Sendable {
    let answer: String
    let hang: Bool
    let first: String?
    var error: Error?
    private(set) var asked: [UInt32] = []
    private(set) var runs: [FakeBlockRun] = []

    init(answer: String, hang: Bool = false, first: String? = nil) {
        self.answer = answer
        self.hang = hang
        self.first = first
    }

    func start(_ session: ExpansionSession, block: UInt32) async throws -> AiBlockRunProtocol {
        try await MainActor.run {
            asked.append(block)
            if let error { throw error }
            let run = FakeBlockRun(block: block, answer: answer, hang: hang, first: first)
            runs.append(run)
            return run
        }
    }
}

private final class FakeBlockRun: AiBlockRunProtocol, @unchecked Sendable {
    private let lock = NSLock()
    private let index: UInt32
    private var pieces: [String]
    private let hang: Bool
    private var arrived = ""
    private var waiting: CheckedContinuation<String?, Never>?
    private var wasCancelled = false

    init(block: UInt32, answer: String, hang: Bool, first: String?) {
        index = block
        self.hang = hang
        if hang {
            pieces = first.map { [$0] } ?? []
        } else {
            let middle = answer.index(answer.startIndex, offsetBy: answer.count / 2)
            pieces = [String(answer[..<middle]), String(answer[middle...])]
        }
    }

    var cancelled: Bool { lock.withLock { wasCancelled } }
    var isWaiting: Bool { lock.withLock { waiting != nil } }

    func next() async throws -> String? {
        let piece = lock.withLock { () -> String? in
            guard !pieces.isEmpty else { return nil }
            let piece = pieces.removeFirst()
            arrived += piece
            return piece
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

    func answer() -> String? {
        let text = lock.withLock { arrived }.trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }

    func block() -> UInt32 { index }
    func cutShort() -> Bool { false }
    func model() -> String { "model-a" }
    func profile() -> String { "Example" }
    func sent() -> [AiContextSent] { [AiContextSent(kind: .fillins, bytes: 9)] }
    func text() -> String { lock.withLock { arrived } }
}
