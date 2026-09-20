import AraloBridge
@testable import AraloKit
import Carbon.HIToolbox
import XCTest

/// Swift calling Rust: the real engine and library behind the generated
/// bindings, with only the operating system faked.
final class BridgeTests: XCTestCase {
    private var folder: URL!
    private var core: Core!
    private var sink: RecordingSink!
    private var controller: ExpansionController!

    override func setUpWithError() throws {
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(UUID().uuidString)")
        core = try Core.openLibrary(path: folder.path)
        core.engine().setFrontApp(bundleId: "com.apple.TextEdit")
        sink = RecordingSink()
        let injector = Injector(sink: sink, pasteboard: FakePasteboard(), sleep: { _ in })
        // Run injection inline so the test can read the result straight away.
        controller = ExpansionController(engine: core.engine(), injector: injector) { $0() }
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: folder)
    }

    /// Types text the way the tap reports it. Returns the characters the app
    /// would have received (the ones that were not consumed).
    @discardableResult
    private func type(_ text: String) -> String {
        var passed = ""
        for character in text {
            let stroke = KeyStroke(keyCode: CGKeyCode(kVK_ANSI_A), text: String(character))
            if !controller.handle(.key(stroke)) {
                passed.append(character)
            }
        }
        return passed
    }

    func testTheCoreAnswersAcrossTheBridge() {
        XCTAssertFalse(coreVersion().isEmpty)
        XCTAssertTrue(excludedAppPresets().contains("com.bitwarden.desktop"))
        XCTAssertGreaterThanOrEqual(core.snippets().count, 9)
        XCTAssertTrue(FileManager.default.fileExists(atPath: folder.appendingPathComponent("aralo.yaml").path))
    }

    func testAStarterSnippetExpands() {
        // The delimiter is consumed and comes back as part of the typed text.
        XCTAssertEqual(type("Ty "), "Ty")
        XCTAssertEqual(sink.keys, [.backspace, .backspace, .text("Thank you ")])
    }

    func testUndoRightAfterAnExpansionRetypesTheAbbreviation() {
        type("ty ")
        let undo = KeyStroke(keyCode: CGKeyCode(kVK_ANSI_Z), flags: .maskCommand, text: "z")
        XCTAssertTrue(controller.handle(.key(undo)))
        XCTAssertEqual(sink.typedText, "ty ")
    }

    /// A controller that translates keys itself, on the US layout, the way
    /// the app does.
    @MainActor
    private func controllerOnTheUSLayout(_ translator: KeyTranslator? = nil) throws -> ExpansionController {
        let layout = try XCTUnwrap(systemLayout("com.apple.keylayout.US"))
        let injector = Injector(sink: sink, pasteboard: FakePasteboard(), sleep: { _ in })
        let translator = translator ?? KeyTranslator(layout: layout)
        return ExpansionController(engine: core.engine(), injector: injector, translator: translator) { $0() }
    }

    /// Presses keys by key code. The event's own string is left wrong on
    /// purpose: with a layout, the controller must not use it.
    private func press(_ keys: [(Int, CGEventFlags)], on controller: ExpansionController) -> [Bool] {
        keys.map { controller.handle(.key(KeyStroke(keyCode: CGKeyCode($0.0), flags: $0.1, text: "?"))) }
    }

    @MainActor
    func testAnAbbreviationWithAnAccentMatchesWhenTypedWithADeadKey() throws {
        try "---\nabbr: né\n---\nnée".write(
            to: folder.appendingPathComponent("nee.md"), atomically: true, encoding: .utf8
        )
        try core.reload()
        let consumed = press(
            [(kVK_ANSI_N, []), (kVK_ANSI_E, .maskAlternate), (kVK_ANSI_E, []), (kVK_Space, [])],
            on: try controllerOnTheUSLayout()
        )
        XCTAssertEqual(consumed, [false, false, false, true])
        XCTAssertEqual(sink.keys, [.backspace, .backspace, .text("née ")])
    }

    @MainActor
    func testBackspaceOverAWaitingAccentForgetsWhatWasTyped() throws {
        let controller = try controllerOnTheUSLayout()
        // "t", the accent, Backspace (removes the accent, not the "t"), "y":
        // the document says "ty", but Aralo can no longer be sure of it.
        let consumed = press(
            [(kVK_ANSI_T, []), (kVK_ANSI_E, .maskAlternate), (kVK_Delete, []), (kVK_ANSI_Y, []), (kVK_Space, [])],
            on: controller
        )
        XCTAssertEqual(consumed, [false, false, false, false, false])
        XCTAssertTrue(sink.keys.isEmpty)
    }

    @MainActor
    func testNothingIsMatchedWhileAnInputMethodComposes() throws {
        let layout = try XCTUnwrap(systemLayout("com.apple.keylayout.US"))
        let translator = KeyTranslator(layout: layout, composing: true)
        let controller = try controllerOnTheUSLayout(translator)
        let keys: [(Int, CGEventFlags)] = [(kVK_ANSI_T, []), (kVK_ANSI_Y, []), (kVK_Space, [])]
        XCTAssertEqual(press(keys, on: controller), [false, false, false])
        XCTAssertTrue(core.engine().holdsNoKeystrokes())

        translator.replace(layout: layout, composing: false)
        XCTAssertEqual(press(keys, on: controller), [false, false, true])
    }

    func testAMouseClickOrAnArrowKeyForgetsWhatWasTyped() {
        type("t")
        XCTAssertFalse(controller.handle(.mouseDown))
        XCTAssertTrue(core.engine().holdsNoKeystrokes())
        type("y ")
        XCTAssertTrue(sink.keys.isEmpty)

        type("t")
        let left = KeyStroke(keyCode: CGKeyCode(kVK_LeftArrow), text: "\u{F702}")
        XCTAssertFalse(controller.handle(.key(left)))
        type("y ")
        XCTAssertTrue(sink.keys.isEmpty)
    }

    func testPausedMeansEveryKeyPasses() {
        core.engine().setPaused(paused: true)
        XCTAssertEqual(type("ty "), "ty ")
        XCTAssertTrue(sink.keys.isEmpty)
    }

    func testEditingTheFolderAndReloadingChangesWhatExpands() throws {
        let file = folder.appendingPathComponent("hello.md")
        try "---\nabbr: ;hi\n---\nhello there".write(to: file, atomically: true, encoding: .utf8)
        try core.reload()
        type(";hi ")
        XCTAssertEqual(sink.typedText, "hello there ")
    }

    /// Spike S2, as a guard rail: a key that matches nothing must cost far less
    /// than the tap's 1 ms budget, bridge included.
    func testOnKeyAcrossTheBridgeIsFarInsideTheBudget() {
        let engine = core.engine()
        let keys = Array("the quick brown fox jumps over the lazy dog ".unicodeScalars).map {
            KeyInput.char(scalar: $0.value)
        }
        let rounds = 5_000
        let start = DispatchTime.now().uptimeNanoseconds
        for _ in 0..<rounds {
            for key in keys { _ = engine.onKey(key: key) }
        }
        let perKey = Double(DispatchTime.now().uptimeNanoseconds - start) / Double(rounds * keys.count)
        print("on_key across the bridge: \(Int(perKey)) ns per key")
        XCTAssertLessThan(perKey, 100_000, "on_key costs \(perKey) ns; the tap budget is 1 ms")
    }
}
