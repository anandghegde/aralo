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
    /// AI settings of the test's own, with keys in memory. Search by meaning
    /// follows their switch (plan 4.10).
    private var settings: AiProfiles!

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
        settings = try AiProfiles.open(path: cache.appendingPathComponent("profiles.toml").path, keys: .memory)
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
        settings = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    func testThePaletteFindsASnippetByWhatItMeansOnceTheModelIsIn() {
        let store = PaletteStore(core: core, inserter: RecordingInserter())
        store.reset()
        store.query = "world hello"
        XCTAssertTrue(store.rows.isEmpty, "no word of the query is in either snippet")

        try? settings.setSwitches(switches: AiSwitches(enabled: true, localOnly: false))
        core.useModel(folder: Self.tinyModel.path, ai: settings)
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
        try? settings.setSwitches(switches: AiSwitches(enabled: true, localOnly: false))
        core.useModel(folder: folder.appendingPathComponent("no-such-model").path, ai: settings)
        core.waitForIndex()
        XCTAssertNotNil(core.meaningProblem())
        let hits = core.search(
            query: SearchQuery(text: "greeting", group: nil, tag: nil, enabledOnly: false, limit: 0, kind: nil)
        )
        XCTAssertEqual(hits.map(\.field), [.label])
    }

    func testTheModelFollowsTheAISwitch() throws {
        // Off, which is how AI starts: the model is not loaded, and says why.
        core.useModel(folder: Self.tinyModel.path, ai: settings)
        core.waitForIndex()
        XCTAssertEqual(fieldsFound(), [])
        XCTAssertEqual(core.meaningProblem(), "AI is switched off")

        try settings.setSwitches(switches: AiSwitches(enabled: true, localOnly: false))
        core.waitForIndex()
        XCTAssertEqual(fieldsFound().first, .meaning)

        // Off again: gone at once, without waiting for the indexer.
        try settings.setSwitches(switches: AiSwitches(enabled: false, localOnly: false))
        XCTAssertEqual(fieldsFound(), [])
    }

    /// Where search found anything for "world hello".
    private func fieldsFound() -> [SearchField] {
        let query = SearchQuery(text: "world hello", group: nil, tag: nil, enabledOnly: false, limit: 0, kind: nil)
        return core.search(query: query).map(\.field)
    }
}
