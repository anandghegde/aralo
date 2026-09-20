import AraloBridge
@testable import AraloHarness
import XCTest

/// The files the harness runs on, as they are in the repository.
final class DataTests: XCTestCase {
    private static let root: URL = {
        var url = URL(fileURLWithPath: #filePath)
        // <root>/apps/macos/Packages/AraloHarness/Tests/AraloHarnessTests/<this file>
        for _ in 0..<7 { url.deleteLastPathComponent() }
        return url
    }()

    private func file(_ path: String) -> URL { Self.root.appendingPathComponent(path) }

    func testEveryAppOfTheTableHasARecipeAndNoRecipeIsLeftOver() throws {
        let book = try RecipeBook.load(file("data/compat/matrix.json"))
        let apps = try Set(compatApps(path: nil).map { $0.bundleId.lowercased() })
        let recipes = Set(book.recipes.map { $0.bundleID.lowercased() })
        XCTAssertEqual(recipes, apps)
        XCTAssertEqual(book.recipe(for: "COM.APPLE.TERMINAL").clearChords, [.killLine])
        XCTAssertEqual(book.recipe(for: "com.apple.mail").chords.count, 4)
        XCTAssertEqual(book.recipe(for: "com.example.unheard-of").open, .manual)
    }

    func testABadRecipeFileIsRefused() {
        func parse(_ json: String) throws -> RecipeBook { try RecipeBook.parse(Data(json.utf8)) }
        XCTAssertThrowsError(try parse(#"{"version": 1, "recipes": []}"#)) {
            XCTAssertEqual($0 as? RecipeBook.Problem, .newerVersion(1))
        }
        let twice = #"{"bundle_id": "a.b", "open": "manual"}"#
        XCTAssertThrowsError(try parse(#"{"version": 0, "recipes": [\#(twice), \#(twice)]}"#)) {
            XCTAssertEqual($0 as? RecipeBook.Problem, .duplicate("a.b"))
        }
        let badKey = #"{"bundle_id": "a.b", "open": "keys", "keys": ["cmd+"]}"#
        XCTAssertThrowsError(try parse(#"{"version": 0, "recipes": [\#(badKey)]}"#)) {
            XCTAssertEqual($0 as? RecipeBook.Problem, .badKey(bundleID: "a.b", key: "cmd+"))
        }
        XCTAssertThrowsError(try parse(#"{"version": 0, "recipes": [{"bundle_id": "a.b", "open": "magic"}]}"#))
    }

    func testTheCoreSaysWhatEachCaseExpandsTo() throws {
        let core = try Core.openLibrary(path: file("fixtures/matrix/library").path)
        let engine = core.engine()

        engine.setFrontApp(bundleId: "com.apple.TextEdit")
        let ascii = try Expectation.ask(engine, abbreviation: MatrixCase.ascii.abbreviation)
        XCTAssertEqual(ascii.typed, "mxascii ")
        XCTAssertEqual(ascii.text, "The quick brown fox jumps over the lazy dog. ")
        XCTAssertEqual(ascii.method, .typed)
        XCTAssertTrue(ascii.undoable)

        let long = try Expectation.ask(engine, abbreviation: MatrixCase.long.abbreviation)
        XCTAssertEqual(long.text.count, 2001)
        XCTAssertEqual(long.method, .pasted)
        XCTAssertTrue(try Expectation.ask(engine, abbreviation: "mxemoji").text.contains("👩‍💻"))

        // A terminal is typed into, however long the text.
        engine.setFrontApp(bundleId: "com.apple.Terminal")
        XCTAssertEqual(try Expectation.ask(engine, abbreviation: "mxlong").method, .typed)

        XCTAssertThrowsError(try Expectation.ask(engine, abbreviation: "nosuchthing"))
        XCTAssertTrue(engine.holdsNoKeystrokes())
    }

    func testEveryCaseHasASnippetInTheMatrixLibrary() throws {
        let core = try Core.openLibrary(path: file("fixtures/matrix/library").path)
        let abbreviations = Set(core.snippets().flatMap(\.abbreviations))
        XCTAssertEqual(abbreviations, Set(MatrixCase.allCases.map(\.abbreviation)))
    }

    func testAForcedMethodIsATableTheCoreAccepts() throws {
        XCTAssertNil(CompatConfig.chosen.table())
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        for config in [CompatConfig.type, .paste] {
            let table = try XCTUnwrap(config.table(undo: "backspace"))
            let path = folder.appendingPathComponent("\(config.rawValue).toml")
            try Data(table.utf8).write(to: path)
            let core = try Core.openLibrary(path: file("fixtures/matrix/library").path)
            try core.loadCompatTable(path: path.path)
            core.engine().setFrontApp(bundleId: "com.apple.Terminal")
            let profile = core.engine().injectionProfile()
            XCTAssertEqual(profile.insert, config == .type ? .type : .paste)
            XCTAssertEqual(profile.undo, .backspace)
        }
    }
}
