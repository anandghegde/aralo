import AraloBridge
@testable import AraloHarness
import XCTest

@MainActor
final class MatrixRunnerTests: XCTestCase {
    private let long = String(repeating: "0123456789", count: 200)
    private var desktop: FakeDesktop!
    private var expectations: [MatrixCase: Expectation] = [:]

    override func setUp() async throws {
        let desktop = FakeDesktop()
        desktop.snippets = ["mxascii ": "The quick brown fox. ", "mxemoji ": "Café 👩‍💻 ", "mxlong ": long + " "]
        self.desktop = desktop
        let quick = Expectation(typed: "mxascii ", text: "The quick brown fox. ", method: .typed)
        let pasted = Expectation(typed: "mxlong ", text: long + " ", method: .pasted)
        expectations = [
            .ascii: quick, .undo: quick, .long: pasted, .clipboard: pasted,
            .unicode: Expectation(typed: "mxemoji ", text: "Café 👩‍💻 ", method: .typed)
        ]
    }

    private func matrix(_ cases: [MatrixCase] = MatrixCase.allCases, recipe: Recipe? = nil) -> AppResult {
        MatrixRunner(desktop: desktop).run(
            app(), recipe: recipe ?? Recipe(bundleID: "com.example.Editor", open: .document),
            cases: cases, expectations: expectations
        )
    }

    func testAWorkingAppPassesEveryCaseAndLeavesTheFieldEmpty() {
        let result = matrix()
        XCTAssertTrue(result.passed)
        XCTAssertEqual(result.cases.map(\.status), [.pass, .pass, .pass, .pending, .pass, .pass])
        XCTAssertEqual(result.cases.map(\.method), ["type", "type", "paste", nil, "type", "paste"])
        XCTAssertEqual(desktop.field, "")
        XCTAssertEqual(desktop.dismissed, ["com.example.Editor"])
    }

    func testLatencyIsTheTimeFromTheDelimiterToTheText() throws {
        desktop.expansionDelay = 0.03
        let latency = try XCTUnwrap(matrix([.ascii]).cases[0].latencyMs)
        XCTAssertEqual(latency, 30, accuracy: 3)
        XCTAssertNil(matrix([.undo]).cases[0].latencyMs)
    }

    func testAFailureCountsOnlyAfterThreeInARow() {
        desktop.deafExpansions = 2
        let flaky = matrix([.ascii]).cases[0]
        XCTAssertEqual(flaky.status, .flaky)
        XCTAssertEqual(flaky.attempts, 3)
        XCTAssertNil(flaky.latencyMs, "a retried case is not a latency sample")

        desktop.deafExpansions = 3
        let failed = matrix([.ascii])
        XCTAssertEqual(failed.cases[0].status, .fail)
        XCTAssertEqual(failed.cases[0].reason, "the abbreviation was left as typed")
        XCTAssertFalse(failed.passed)
    }

    func testAnAppThatTellsAccessibilityNothingIsReadByCopying() {
        desktop.speaksAccessibility = false
        desktop.clipboard = "what the person had copied"
        let result = matrix([.ascii, .undo])
        XCTAssertEqual(result.cases.map(\.status), [.pass, .pass])
        XCTAssertEqual(result.cases[0].readBack, .copy)
        XCTAssertNil(result.cases[0].latencyMs, "a copy says nothing about when the text arrived")
        XCTAssertEqual(desktop.clipboard, "what the person had copied")
    }

    func testAClipboardThatIsNotPutBackFails() {
        desktop.restoresClipboard = false
        let result = matrix([.clipboard]).cases[0]
        XCTAssertEqual(result.status, .fail)
        XCTAssertEqual(result.reason, "the clipboard was not put back")
    }

    func testUndoMustBringTheAbbreviationBack() {
        desktop.undoWorks = false
        let result = matrix([.undo]).cases[0]
        XCTAssertEqual(result.status, .fail)
        XCTAssertEqual(result.reason, "undo did not bring the abbreviation back")
    }

    func testNoKeyIsSentOnceAnotherAppHasTheKeyboard() {
        desktop.stealsFocusAfterKeys = 5
        let result = matrix()
        XCTAssertEqual(result.cases.map(\.status), [.focusLost])
        XCTAssertFalse(result.passed)
        let sent = desktop.keysSent
        XCTAssertLessThanOrEqual(sent, "mxascii ".count + 3, "the rest of the word at most, never a new chord")
    }

    func testAFieldThatAlreadyHasTextIsLeftAlone() {
        desktop.field = "Somebody's unsaved letter."
        let result = matrix()
        XCTAssertEqual(result.skipped, "the field with the keyboard already has text in it")
        XCTAssertFalse(result.tested)
        XCTAssertEqual(desktop.keysSent, 0)
        XCTAssertEqual(desktop.field, "Somebody's unsaved letter.")
        XCTAssertEqual(desktop.dismissed, ["com.example.Editor"])
    }

    func testATerminalIsClearedWithKillLine() {
        let result = matrix([.ascii], recipe: Recipe(bundleID: "com.example.Editor", open: .shell, clear: .killLine))
        XCTAssertTrue(result.passed)
        XCTAssertEqual(desktop.field, "")
    }

    func testAnAppThatIsNotThereIsSkippedNotFailed() {
        desktop.bringUpResult = .notInstalled
        let result = matrix()
        XCTAssertEqual(result.skipped, "not installed")
        XCTAssertFalse(result.tested)
        XCTAssertFalse(result.passed)
        XCTAssertEqual(desktop.keysSent, 0)
    }

    func testThePendingCaseRunsWhenAskedFor() {
        desktop.snippets["mxcursor "] = "Dear , thanks "
        expectations[.cursor] = Expectation(typed: "mxcursor ", text: "Dear , thanks ", caretBack: 9, method: .typed)
        var options = MatrixRunner.Options()
        options.includePending = true
        let runner = MatrixRunner(desktop: desktop, options: options)
        let result = runner.run(
            app(), recipe: Recipe(bundleID: "com.example.Editor", open: .document),
            cases: [.cursor], expectations: expectations
        )
        // The fake field has no caret, so "x" lands at the end: a caret in the wrong place.
        XCTAssertEqual(result.cases[0].status, .fail)
        XCTAssertEqual(result.cases[0].reason, "the caret was not where the marker is")
        XCTAssertEqual(expectations[.cursor]?.text(withAtCaret: "x"), "Dear x, thanks ")
    }
}
