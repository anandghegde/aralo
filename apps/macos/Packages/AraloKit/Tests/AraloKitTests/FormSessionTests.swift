import AraloBridge
@testable import AraloKit
import XCTest

/// The form panel's model, against the real core: a snippet that asks
/// something, driven the way a panel drives it. What happens to the plan
/// afterwards is the injector's business and is measured in BridgeTests.
@MainActor
final class FormSessionTests: XCTestCase {
    private var folder: URL!
    private var cache: URL!
    private var core: Core!
    private var runner: RecordingRunner!
    private var clipboard: ClipboardStub!

    private enum Problem: Error {
        /// The engine expanded instead of asking, which is the test's fault:
        /// the body has no placeholder that needs an answer.
        case nothingWasAsked
    }

    override func setUpWithError() throws {
        let run = UUID().uuidString
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)")
        cache = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)-cache")
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil, trash: nil)
        core.engine().setFrontApp(bundleId: "com.apple.TextEdit")
        runner = RecordingRunner()
        clipboard = ClipboardStub()
    }

    override func tearDownWithError() throws {
        runner = nil
        core = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    /// A snippet, picked from the palette: nothing was typed to ask for it, so
    /// a cancelled session has nothing to put back.
    private func picked(_ body: String, label: String = "Ticket") throws -> FormSession {
        let id = try core.createSnippet(
            group: [],
            draft: SnippetDraft(
                label: label, abbreviations: [";tkt"], body: body, tags: [], kind: .text,
                trigger: nil, case: nil, wholeWord: nil, keepDelimiter: nil, enabled: nil
            )
        )
        try core.reload()
        guard case .startSession(_, let session) = core.engine()
            .insert(snippetId: id, intoApp: "com.apple.TextEdit")
        else {
            throw Problem.nothingWasAsked
        }
        return FormSession(
            session: session, runner: runner, label: label, clipboard: { [clipboard] in clipboard?.read() }
        )
    }

    // MARK: - Asking

    func testAFormOpensOnItsDefaultsAndPreviewsThem() throws {
        let form = try picked("Dear {{field: who | label: Their name | default: friend}},")

        XCTAssertEqual(form.fields.map(\.name), ["who"])
        XCTAssertEqual(form.fields.first?.label, "Their name")
        XCTAssertEqual(form.answers, ["who": "friend"])
        // The preview is the expansion, so what it shows is what Enter types.
        XCTAssertEqual(form.preview, "Dear friend,")
        XCTAssertFalse(form.isFinished)
        // Nothing has been typed anywhere: asking is free.
        XCTAssertTrue(runner.actions.isEmpty)
        XCTAssertEqual(form.label, "Ticket")
    }

    func testTypingAnAnswerRedrawsThePreview() throws {
        let form = try picked("Dear {{field: who | default: friend}},")

        form.answers["who"] = "Da"
        XCTAssertEqual(form.preview, "Dear Da,")
        form.answers["who"] = "Dana"
        XCTAssertEqual(form.preview, "Dear Dana,")
        // Still nothing typed into the document, and the form still open.
        XCTAssertTrue(runner.actions.isEmpty)
        XCTAssertFalse(form.isFinished)
    }

    func testADropDownStartsOnItsFirstChoice() throws {
        let form = try picked("Sent by {{choice: how | options: post, e-mail}}.")

        XCTAssertEqual(form.fields.first?.options, ["post", "e-mail"])
        XCTAssertEqual(form.answers, ["how": "post"])
        XCTAssertEqual(form.preview, "Sent by post.")
    }

    // MARK: - Answering

    func testSubmittingHandsOverThePlanAndEndsTheSession() throws {
        let form = try picked("Dear {{field: who | default: friend}},")
        form.answers["who"] = "Dana"

        form.submit()

        XCTAssertEqual(runner.actions.count, 1)
        XCTAssertEqual(insertedText(runner.actions.first), "Dear Dana,")
        XCTAssertTrue(form.isFinished)
        // A panel that submits twice, because Enter arrived twice, types once.
        form.submit()
        XCTAssertEqual(runner.actions.count, 1)
    }

    func testTheClipboardIsFetchedForABodyThatAsksForItAndOnlyThen() throws {
        clipboard.text = "PO-8841"
        let form = try picked("About {{clipboard}} for {{field: who}}.")
        // Nothing is read while the form is open: the pasteboard is fetched
        // when the answers are in, which is when the expansion happens.
        XCTAssertEqual(clipboard.reads, 0)
        XCTAssertEqual(form.preview, "About {{clipboard}} for .")

        form.answers["who"] = "Dana"
        form.submit()

        XCTAssertEqual(clipboard.reads, 1)
        XCTAssertEqual(insertedText(runner.actions.first), "About PO-8841 for Dana.")
    }

    func testABodyThatAsksForNothingOutsideNeverTouchesThePasteboard() throws {
        let form = try picked("Dear {{field: who | default: friend}},")

        form.submit()

        XCTAssertEqual(clipboard.reads, 0)
        XCTAssertEqual(insertedText(runner.actions.first), "Dear friend,")
    }

    /// A body that wants only the clipboard is not a body with a question for
    /// the user: it expands where it stands, and no panel ever appears.
    func testASnippetThatOnlyWantsTheClipboardNeedsNoPanel() throws {
        clipboard.text = "PO-8841"
        let form = try picked("Order {{clipboard}} has shipped.")

        XCTAssertTrue(form.isFinished)
        XCTAssertTrue(form.fields.isEmpty)
        XCTAssertEqual(insertedText(runner.actions.first), "Order PO-8841 has shipped.")
    }

    /// A pasteboard with nothing on it was still read, so the placeholder is
    /// worth the empty string. A user who copied an image must not find
    /// `{{clipboard}}` typed into their document.
    func testAPasteboardWithNoTextOnItExpandsToNothing() throws {
        clipboard.text = nil
        let form = try picked("Order {{clipboard}} has shipped.")

        XCTAssertTrue(form.isFinished)
        XCTAssertEqual(clipboard.reads, 1)
        XCTAssertEqual(insertedText(runner.actions.first), "Order  has shipped.")
    }

    func testCancellingTypesNothingAndHandsTheSessionBack() throws {
        let form = try picked("Dear {{field: who | default: friend}},")

        form.cancel()

        XCTAssertTrue(form.isFinished)
        XCTAssertEqual(runner.cancelled, 1)
        XCTAssertTrue(runner.actions.isEmpty)
        // Cancelled twice is still cancelled once, and submitting afterwards
        // types nothing: the user said no.
        form.cancel()
        form.submit()
        XCTAssertEqual(runner.cancelled, 1)
        XCTAssertTrue(runner.actions.isEmpty)
    }

    /// The text an `.expand` action would insert.
    private func insertedText(_ action: SessionAction?) -> String {
        guard case .expand(_, let steps, _, _) = action else { return "" }
        return steps.reduce(into: "") { text, step in
            if case .insertText(let inserted) = step { text += inserted }
        }
    }
}

/// A runner that writes down what it was handed instead of typing it.
final class RecordingRunner: ExpansionRunner, @unchecked Sendable {
    private(set) var actions: [SessionAction] = []
    private(set) var cancelled = 0

    func run(_ action: SessionAction) {
        actions.append(action)
    }

    func cancel(_ session: ExpansionSession) {
        cancelled += 1
    }
}

/// A pasteboard that counts how often it was read: a snippet that did not ask
/// for the clipboard must not cause it to be read.
@MainActor
final class ClipboardStub {
    var text: String?
    private(set) var reads = 0

    func read() -> String? {
        reads += 1
        return text
    }
}
