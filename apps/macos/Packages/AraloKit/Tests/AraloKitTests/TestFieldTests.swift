import AraloBridge
@testable import AraloKit
import XCTest

/// The editor's test field, against the real core and a real folder. Each test
/// types into it and reads what the text view would draw.
@MainActor
final class TestFieldTests: XCTestCase {
    private var folder: URL!
    private var cache: URL!
    private var core: Core!
    private var store: LibraryStore!
    /// Every claim the field made on the keyboard, in order.
    private var claims: Claims!

    override func setUpWithError() throws {
        let run = UUID().uuidString
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)")
        cache = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)-cache")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try "format: 0\n".write(
            to: folder.appendingPathComponent("aralo.yaml"), atomically: true, encoding: .utf8
        )
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil, trash: nil)
        store = LibraryStore(core: core)
        let claims = Claims()
        self.claims = claims
        store.onTestFieldKeyboard = { claims.made.append($0) }
    }

    override func tearDownWithError() throws {
        store = nil
        core = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    /// Opens a saved snippet in the editor, then puts `body` on screen without
    /// saving it.
    private func editing(_ abbreviation: String, _ body: String) throws -> TestField {
        let draft = SnippetDraft(
            label: "Greeting", abbreviations: [abbreviation], body: "saved", tags: [], kind: .text,
            trigger: nil, case: nil, wholeWord: nil, keepDelimiter: nil, enabled: nil
        )
        let id = try core.createSnippet(group: [], draft: draft)
        store.refresh()
        store.select(snippet: id)
        store.editing?.draft.body = body
        return try XCTUnwrap(store.testField())
    }

    func testTheDraftOnScreenExpandsNotTheFile() throws {
        let field = try editing(";hi", "Hello there")
        XCTAssertEqual(field.abbreviations, 1)
        field.type("so ;hi ")
        XCTAssertEqual(field.field.text, "so Hello there ")
        XCTAssertEqual(field.field.caret, 15)
        XCTAssertEqual(store.editing?.isDirty, true, "trying the draft saved it")
    }

    func testReturnIsADelimiterAsItIsEverywhere() throws {
        let field = try editing(";hi", "Hello")
        field.type("x\n;hi\n")
        XCTAssertEqual(field.field.text, "x\nHello\n")
    }

    func testBackspaceTakesTheExpansionBack() throws {
        let field = try editing(";hi", "Hello")
        field.type(";hi ")
        field.type(.backspace)
        XCTAssertEqual(field.field.text, ";hi ")
    }

    func testMovingTheCaretBreaksAnAbbreviationInTwo() throws {
        let field = try editing(";hi", "Hello")
        field.type(";h")
        field.moveCaret(to: 0)
        field.moveCaret(to: 2)
        field.type("i ")
        XCTAssertEqual(field.field.text, ";hi ")
    }

    func testAFormOpensAndWhatItComesToLandsInTheField() throws {
        let field = try editing(";hi", "Hi {{field: who | default: friend}}.")
        field.type(";hi ")
        let form = try XCTUnwrap(field.form)
        XCTAssertEqual(form.label, "Greeting")
        XCTAssertEqual(field.field.text, ";hi", "the space waits with the form")

        field.type("x")
        XCTAssertEqual(field.field.text, ";hi", "keys go to the form while it is up")

        form.answers["who"] = "Dana"
        field.submitForm()
        XCTAssertNil(field.form)
        XCTAssertEqual(field.field.text, "Hi Dana. ")
    }

    func testACancelledFormPutsTheKeyBack() throws {
        let field = try editing(";hi", "Hi {{field: who}}.")
        field.type(";hi ")
        field.cancelForm()
        XCTAssertNil(field.form)
        XCTAssertEqual(field.field.text, ";hi ")
    }

    func testTheKeyboardIsClaimedOnceAndGivenBack() throws {
        let field = try editing(";hi", "Hi {{field: who}}.")
        field.setViewHasKeyboard(true)
        field.type(";hi ")
        // The view loses the keyboard to the form; the claim carries over.
        field.setFormHasKeyboard(true)
        field.setViewHasKeyboard(false)
        field.submitForm()
        XCTAssertEqual(claims.made, [true, false])

        field.setViewHasKeyboard(true)
        field.close()
        XCTAssertEqual(claims.made, [true, false, true, false])
    }

    func testClosingCancelsAnOpenForm() throws {
        let field = try editing(";hi", "Hi {{field: who}}.")
        field.type(";hi ")
        field.close()
        XCTAssertNil(field.form)
        XCTAssertEqual(field.field.text, ";hi ")
    }

    func testADraftWithNothingToTypeSaysSo() throws {
        let field = try editing("", "Hello")
        XCTAssertEqual(field.abbreviations, 0)
    }
}

@MainActor
private final class Claims {
    var made: [Bool] = []
}
