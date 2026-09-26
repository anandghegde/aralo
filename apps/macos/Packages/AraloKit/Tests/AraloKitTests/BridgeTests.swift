import AraloBridge
@testable import AraloKit
import Carbon.HIToolbox
import XCTest

/// Swift calling Rust: the real engine and library behind the generated
/// bindings, with only the operating system faked.
final class BridgeTests: XCTestCase {
    private var folder: URL!
    /// Outside the library, so the watch never sees it, and inside the
    /// temporary folder, so a test run never touches the real one.
    private var cache: URL!
    private var core: Core!
    private var sink: RecordingSink!
    private var controller: ExpansionController!

    override func setUpWithError() throws {
        let run = UUID().uuidString
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)")
        cache = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)-cache")
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil, trash: nil)
        core.engine().setFrontApp(bundleId: "com.apple.TextEdit")
        sink = RecordingSink()
        let injector = Injector(sink: sink, pasteboard: FakePasteboard(), sleep: { _ in })
        // Run injection inline so the test can read the result straight away.
        controller = ExpansionController(engine: core.engine(), injector: injector) { $0() }
    }

    override func tearDownWithError() throws {
        // Let go of the core before the folder goes: dropping it stops the
        // threads that are watching and indexing it.
        controller = nil
        core = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
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

    /// A password field turns secure input on. The monitor the app runs sees
    /// the system's own flag, and what was typed before it finishes nothing.
    @MainActor
    func testSecureInputGoingOnForgetsWhatWasTyped() throws {
        try XCTSkipIf(IsSecureEventInputEnabled(), "something on this machine holds secure input")
        var heard: [Bool] = []
        let monitor = SecureInputMonitor(resetting: core.engine()) { heard.append($0) }
        type("t")
        XCTAssertFalse(core.engine().holdsNoKeystrokes())

        // What NSSecureTextField does when it takes the keyboard.
        XCTAssertEqual(EnableSecureEventInput(), noErr)
        defer { DisableSecureEventInput() }
        XCTAssertTrue(IsSecureEventInputEnabled())
        monitor.start(interval: 60)
        monitor.stop()

        XCTAssertEqual(heard, [true])
        XCTAssertTrue(core.engine().holdsNoKeystrokes())
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

    // MARK: - A snippet picked from a list

    /// Adds a snippet and answers with its ID, for the tests that insert one
    /// without typing anything.
    @discardableResult
    private func add(_ label: String, _ abbreviation: String, body: String) throws -> String {
        let id = try core.createSnippet(
            group: [],
            draft: SnippetDraft(
                label: label, abbreviations: [abbreviation], body: body, tags: [], kind: .text,
                trigger: nil, case: nil, wholeWord: nil, keepDelimiter: nil, enabled: nil
            )
        )
        try core.reload()
        return id
    }

    func testAPickedSnippetIsTypedWholeWithNothingDeletedFirst() throws {
        let id = try add("Address", ";addr", body: "12 Mill Lane")

        XCTAssertNil(controller.insert(snippetId: id, into: "com.apple.TextEdit"))

        // Nothing was typed to ask for it, so there is nothing to take back.
        XCTAssertEqual(sink.keys, [.text("12 Mill Lane")])
    }

    func testAPickedSnippetIsUndoneByTheUndoKeyAndRetypesNothing() throws {
        let id = try add("Address", ";addr", body: "12 Mill Lane")
        controller.insert(snippetId: id, into: "com.apple.TextEdit")

        let undo = KeyStroke(keyCode: CGKeyCode(kVK_ANSI_Z), flags: .maskCommand, text: "z")
        XCTAssertTrue(controller.handle(.key(undo)))

        // The text goes, and no abbreviation comes back in its place: the user
        // never typed one.
        XCTAssertEqual(sink.typedText, "")
        XCTAssertEqual(sink.keys.filter { $0 == .backspace }.count, 12)
    }

    func testAPickIsRefusedWhilePausedOrForAnAppAraloStaysOutOf() throws {
        let id = try add("Address", ";addr", body: "12 Mill Lane")

        XCTAssertEqual(controller.insert(snippetId: id, into: "com.bitwarden.desktop"), .excludedApp)
        core.engine().setPaused(paused: true)
        XCTAssertEqual(controller.insert(snippetId: id, into: "com.apple.TextEdit"), .paused)
        XCTAssertEqual(controller.insert(snippetId: "not-a-snippet", into: "com.apple.TextEdit"), .snippetGone)

        XCTAssertTrue(sink.keys.isEmpty)
    }

    func testTheKeyboardComingBackTwiceDoesNotCostThePickItsUndo() throws {
        let id = try add("Address", ";addr", body: "12 Mill Lane")
        // What the palette's window does: it gives the keyboard back, inserts,
        // and then says it has gone. The second is not a change, so it must not
        // reset anything: the reset would take the undo record with it.
        controller.setSuspended(true)
        controller.setSuspended(false)
        controller.insert(snippetId: id, into: "com.apple.TextEdit")
        controller.setSuspended(false)

        let undo = KeyStroke(keyCode: CGKeyCode(kVK_ANSI_Z), flags: .maskCommand, text: "z")
        XCTAssertTrue(controller.handle(.key(undo)))
        XCTAssertEqual(sink.typedText, "")
    }

    func testWhatIsTypedIntoTheSearchPaletteIsNotWatchedAndNotRemembered() {
        type("t")
        controller.setSuspended(true)

        // The palette has the keyboard: every key reaches its own field, and
        // the half-typed abbreviation underneath is forgotten.
        XCTAssertEqual(type("y "), "y ")
        XCTAssertTrue(sink.keys.isEmpty)
        XCTAssertTrue(core.engine().holdsNoKeystrokes())

        controller.setSuspended(false)
        // What was typed before the palette opened cannot join what is typed
        // after it closes.
        XCTAssertEqual(type("y "), "y ")
        XCTAssertTrue(sink.keys.isEmpty)
        type("ty ")
        XCTAssertEqual(sink.typedText, "thank you ")
    }

    // MARK: - A snippet that asks something first

    /// A body with a form, and something to catch the session it starts.
    @discardableResult
    private func askingSnippet() throws -> SessionBox {
        try add("Ticket", ";tkt", body: "Dear {{field: who | default: friend}},")
        let held = SessionBox()
        controller.setSessionHandler { held.hold($0) }
        return held
    }

    func testASnippetThatAsksSomethingTypesNothingUntilItIsAnswered() throws {
        let held = try askingSnippet()

        // The delimiter is swallowed: a panel is opening, and the character
        // goes back into the document only if the user changes their mind.
        XCTAssertEqual(type(";tkt "), ";tkt")
        XCTAssertTrue(sink.keys.isEmpty)

        let session = try XCTUnwrap(held.session)
        controller.run(session.submitForm(answers: ["who": "Dana"]))

        // The abbreviation goes, the answer arrives, and the delimiter comes
        // back after it: the same plan a body with no questions would make.
        XCTAssertEqual(sink.keys, [.backspace, .backspace, .backspace, .backspace, .text("Dear Dana, ")])
    }

    func testAnAnsweredFormIsUndoneByTheUndoKeyLikeAnyOtherExpansion() throws {
        let held = try askingSnippet()
        type(";tkt ")
        let session = try XCTUnwrap(held.session)
        controller.run(session.submitForm(answers: ["who": "Dana"]))

        let undo = KeyStroke(keyCode: CGKeyCode(kVK_ANSI_Z), flags: .maskCommand, text: "z")
        XCTAssertTrue(controller.handle(.key(undo)))

        // What the user typed comes back, and what the form filled in goes.
        XCTAssertEqual(sink.typedText, ";tkt ")
    }

    func testAFormTheUserCancelsPutsBackTheKeyThatOpenedIt() throws {
        let held = try askingSnippet()
        type(";tkt ")

        controller.cancel(try XCTUnwrap(held.session))

        // Nothing was inserted, so there is nothing to delete: the one
        // character the match swallowed is typed, and that is all.
        XCTAssertEqual(sink.keys, [.text(" ")])
    }

    /// Without a handler there is nobody to ask, and a swallowed keystroke must
    /// not simply vanish.
    func testASessionNobodyCanDriveIsEndedRatherThanLeftOpen() throws {
        try add("Ticket", ";tkt", body: "Dear {{field: who | default: friend}},")

        XCTAssertEqual(type(";tkt "), ";tkt")
        XCTAssertEqual(sink.keys, [.text(" ")])
    }

    func testAPickedSnippetThatAsksSomethingIsNotTypedUntilItIsAnswered() throws {
        let id = try add("Ticket", ";tkt", body: "Dear {{field: who | default: friend}},")
        let held = SessionBox()
        controller.setSessionHandler { held.hold($0) }

        // Nothing is refused: the snippet has a question, and the panel asks it.
        XCTAssertNil(controller.insert(snippetId: id, into: "com.apple.TextEdit"))
        XCTAssertTrue(sink.keys.isEmpty)

        let session = try XCTUnwrap(held.session)
        controller.run(session.submitForm(answers: [:]))

        // Nothing was typed to ask for it, so there is nothing to take back,
        // and a field nobody filled in keeps its default.
        XCTAssertEqual(sink.keys, [.text("Dear friend,")])
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
