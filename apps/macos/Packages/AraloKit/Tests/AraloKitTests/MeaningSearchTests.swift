import AraloBridge
@testable import AraloKit
import XCTest

/// Search by meaning through the bridge (PRD A4), with the tiny model in
/// `fixtures/embed/tiny`. Its numbers are random, so it knows nothing about
/// meaning, but it averages tokens as the real one does: "world hello" is the
/// same vector as "hello world", and no word search finds one in the other.
@MainActor
final class MeaningSearchTests: XCTestCase {
    private var folder: URL!
    private var cache: URL!
    private var core: Core!

    /// `fixtures/embed/tiny` at the root of the repository.
    private static let tinyModel = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent() // AraloKitTests
        .deletingLastPathComponent() // Tests
        .deletingLastPathComponent() // AraloKit
        .deletingLastPathComponent() // Packages
        .deletingLastPathComponent() // macos
        .deletingLastPathComponent() // apps
        .deletingLastPathComponent()
        .appendingPathComponent("fixtures/embed/tiny")

    override func setUpWithError() throws {
        let run = UUID().uuidString
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)")
        cache = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)-cache")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try "format: 0\n".write(to: folder.appendingPathComponent("aralo.yaml"), atomically: true, encoding: .utf8)
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil, trash: nil)
        for (label, abbreviation, body) in [("Greeting", ";gr", "hello world"), ("Farewell", ";fw", "goodbye moon")] {
            _ = try core.createSnippet(
                group: [],
                draft: SnippetDraft(
                    label: label, abbreviations: [abbreviation], body: body, tags: [], kind: .text,
                    trigger: nil, case: nil, wholeWord: nil, keepDelimiter: nil, enabled: nil
                )
            )
        }
    }

    override func tearDownWithError() throws {
        core = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    func testThePaletteFindsASnippetByWhatItMeansOnceTheModelIsIn() {
        let store = PaletteStore(core: core, inserter: RecordingInserter())
        store.reset()
        store.query = "world hello"
        XCTAssertTrue(store.rows.isEmpty, "no word of the query is in either snippet")

        core.useModel(folder: Self.tinyModel.path)
        core.waitForIndex()
        XCTAssertNil(core.meaningProblem())
        store.query = ""
        store.query = "world hello"
        XCTAssertEqual(store.rows.first?.label, "Greeting")
        XCTAssertEqual(store.rows.first?.field, .meaning)
        XCTAssertEqual(store.rows.first?.detail, "hello world")
        XCTAssertEqual(store.rows.first?.matched, [])
    }

    func testAModelThatWillNotLoadSaysWhyAndSearchKeepsToWords() {
        core.useModel(folder: folder.appendingPathComponent("no-such-model").path)
        core.waitForIndex()
        XCTAssertNotNil(core.meaningProblem())
        let hits = core.search(
            query: SearchQuery(text: "greeting", group: nil, tag: nil, enabledOnly: false, limit: 0, kind: nil)
        )
        XCTAssertEqual(hits.map(\.field), [.label])
    }
}
