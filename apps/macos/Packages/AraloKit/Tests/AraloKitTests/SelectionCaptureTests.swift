import AraloBridge
@testable import AraloKit
import XCTest

/// Reading and replacing the selection for a command (task 4.5), with the
/// real engine behind the controller and the app faked.
@MainActor
final class SelectionCaptureTests: XCTestCase {
    private var folder: URL!
    private var cache: URL!
    private var core: Core!
    private var pasteboard: FakePasteboard!
    private var sink: AppSink!
    private var controller: ExpansionController!
    /// What the app copies when it is sent Cmd+C. Nil copies nothing.
    private var selectedInApp: String?

    override func setUpWithError() throws {
        let run = UUID().uuidString
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)")
        cache = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)-cache")
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil, trash: nil)
        pasteboard = FakePasteboard(items: [["public.utf8-plain-text": Data("mine".utf8)]])
        sink = AppSink { [unowned self] key in
            if key == .copy, let selectedInApp { pasteboard.copyFromElsewhere(selectedInApp) }
        }
        let injector = Injector(sink: sink, pasteboard: pasteboard, sleep: { _ in })
        controller = ExpansionController(engine: core.engine(), injector: injector) { $0() }
    }

    override func tearDownWithError() throws {
        controller = nil
        core = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    private func capture(
        app: String = "com.apple.TextEdit",
        comesBack: Bool = true,
        secureInput: Bool = false,
        accessibility: AccessibilitySelection
    ) -> SelectionCapture {
        SelectionCapture(
            controller: controller,
            target: StubTarget(bundleId: app, comesBack: comesBack),
            secureInput: { secureInput },
            accessibility: { _ in accessibility }
        )
    }

    func testAccessibilityIsReadFirstAndTouchesNothing() async {
        let read = await capture(accessibility: .text("their going home")).read()
        XCTAssertEqual(read, .success("their going home"))
        XCTAssertEqual(sink.keys, [])
        XCTAssertEqual(pasteboard.changeCount, 0)
    }

    func testAnAppThatDoesNotSayIsCopiedFromAndTheClipboardComesBack() async {
        selectedInApp = "copied text"
        let read = await capture(accessibility: .unavailable).read()
        XCTAssertEqual(read, .success("copied text"))
        XCTAssertEqual(sink.keys, [.copy])
        XCTAssertEqual(pasteboard.text(), "mine")

        selectedInApp = nil
        let nothing = await capture(accessibility: .unavailable).read()
        XCTAssertEqual(nothing, .failure(.nothingSelected))
    }

    func testATextFieldWithNothingSelectedIsBelievedAndNotCopiedFrom() async {
        selectedInApp = "the whole line"
        let read = await capture(accessibility: .nothing).read()
        XCTAssertEqual(read, .failure(.nothingSelected))
        XCTAssertEqual(sink.keys, [], "no Cmd+C, which copies a line in some editors")
    }

    func testPasswordsPausedAndExcludedAppsAreNeverRead() async {
        selectedInApp = "secret"
        let field = await capture(accessibility: .passwordField).read()
        XCTAssertEqual(field, .failure(.passwordField))
        let secure = await capture(secureInput: true, accessibility: .unavailable).read()
        XCTAssertEqual(secure, .failure(.passwordField))
        let excluded = await capture(app: "com.bitwarden.desktop", accessibility: .unavailable).read()
        XCTAssertEqual(excluded, .failure(.refused(.excludedApp)))

        core.engine().setPaused(paused: true)
        let paused = await capture(accessibility: .text("anything")).read()
        XCTAssertEqual(paused, .failure(.refused(.paused)))
        XCTAssertEqual(sink.keys, [])
    }

    func testReplacingPastesIntoTheAppOnlyOnceItIsBack() async {
        var pasted: String?
        pasteboard.onWrite = { [unowned self] in pasted = pasteboard.text() }
        let failure = await capture(accessibility: .text("x")).replace(with: "They're going home.")
        XCTAssertNil(failure)
        XCTAssertEqual(sink.keys, [.paste])
        XCTAssertEqual(pasted, "They're going home.")
        XCTAssertEqual(pasteboard.text(), "mine", "the clipboard comes back")

        let lost = await capture(comesBack: false, accessibility: .text("x")).replace(with: "anything")
        XCTAssertEqual(lost, .noTargetApp)
        core.engine().setPaused(paused: true)
        let paused = await capture(accessibility: .text("x")).replace(with: "anything")
        XCTAssertEqual(paused, .refused(.paused))
        XCTAssertEqual(sink.keys, [.paste], "nothing more was sent")
    }
}

@MainActor
private struct StubTarget: TargetApp {
    let bundleId: String
    let comesBack: Bool

    func activate() async -> Bool { comesBack }
}
