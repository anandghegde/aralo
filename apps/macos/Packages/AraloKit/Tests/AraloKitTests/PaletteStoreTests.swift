import AraloBridge
@testable import AraloKit
import XCTest

/// The search palette's model, against the real core and a real folder. The
/// insertion is the one thing faked: what happens to the text is the injector's
/// business and is measured in the harness, not here.
@MainActor
final class PaletteStoreTests: XCTestCase {
    private var folder: URL!
    private var cache: URL!
    private var core: Core!
    private var inserter: RecordingInserter!
    private var store: PaletteStore!

    override func setUpWithError() throws {
        let run = UUID().uuidString
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)")
        cache = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)-cache")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try "format: 0\n".write(
            to: folder.appendingPathComponent("aralo.yaml"), atomically: true, encoding: .utf8
        )
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil, trash: nil)
        inserter = RecordingInserter()
    }

    override func tearDownWithError() throws {
        store = nil
        inserter = nil
        core = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    @discardableResult
    private func write(_ label: String, _ abbreviation: String, body: String, enabled: Bool? = nil) throws -> String {
        try core.createSnippet(
            group: [],
            draft: SnippetDraft(
                label: label, abbreviations: [abbreviation], body: body, tags: [], kind: .text,
                trigger: nil, case: nil, wholeWord: nil, keepDelimiter: nil, enabled: enabled
            )
        )
    }

    /// Opens the palette on the library as it now stands.
    private func open() {
        store = PaletteStore(core: core, inserter: inserter)
        store.reset()
    }

    // MARK: - The list

    func testAnEmptyQueryListsEverythingThatCanExpand() throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        try write("Standup", ";stand", body: "Yesterday:")
        open()

        XCTAssertEqual(Set(store.rows.map(\.label)), ["Address", "Standup"])
        // Something is always ready for Enter, so the palette never opens on a
        // list that does nothing.
        XCTAssertEqual(store.selection, store.rows.first?.id)
    }

    func testTheSnippetUsedLastIsOfferedFirst() throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        let standup = try write("Standup", ";stand", body: "Yesterday:")
        core.engine().expansionDone(snippetId: standup, deleteCount: 6, method: .typed)
        try waitForRecents(toContain: standup)
        open()

        XCTAssertEqual(store.rows.first?.id, standup)
        // The rest of the library is still there, under it.
        XCTAssertEqual(store.rows.count, 2)
    }

    func testASwitchedOffSnippetIsNotOffered() throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        try write("Retired", ";old", body: "an old address", enabled: false)
        open()

        XCTAssertEqual(store.rows.map(\.label), ["Address"])
        store.query = "Retired"
        XCTAssertTrue(store.rows.isEmpty)
        XCTAssertNil(store.selection)
    }

    func testTypingNarrowsTheListAndSelectsTheTopRow() throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        let standup = try write("Standup", ";stand", body: "Yesterday:")
        open()
        store.select(standup)

        store.query = "Address"

        XCTAssertEqual(store.rows.map(\.label), ["Address"])
        // A query that leaves the old selection out moves it to the top rather
        // than pointing at a row nobody can see.
        XCTAssertEqual(store.selection, store.rows.first?.id)
        XCTAssertNotEqual(store.selection, standup)
    }

    func testTheSelectionStaysOnTheSnippetWhenTheQueryKeepsIt() throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        try write("Standup", ";stand", body: "Yesterday:")
        open()
        store.query = "Address"
        let address = store.selection

        store.query = "Addre"

        XCTAssertEqual(store.selection, address)
    }

    func testArrowsMoveThroughTheListAndStopAtItsEnds() throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        try write("Standup", ";stand", body: "Yesterday:")
        open()
        let ids = store.rows.map(\.id)

        store.selectPrevious()
        XCTAssertEqual(store.selection, ids[0], "Up at the top of the list stays at the top")
        store.selectNext()
        XCTAssertEqual(store.selection, ids[1])
        store.selectNext()
        XCTAssertEqual(store.selection, ids[1], "Down at the bottom stays at the bottom")
    }

    func testAClickSelectsARowAndOneOnNothingChangesNothing() throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        let standup = try write("Standup", ";stand", body: "Yesterday:")
        open()

        store.select(standup)
        XCTAssertEqual(store.selection, standup)
        // A click that landed as the list was redrawn selects nothing new
        // rather than emptying the selection.
        store.select("not-a-snippet")
        XCTAssertEqual(store.selection, standup)
    }

    func testTheSelectedSnippetIsPreviewedAsItWillArrive() throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        open()

        XCTAssertEqual(store.preview, "12 Mill Lane")
    }

    // MARK: - Inserting

    func testEnterInsertsTheSelectedSnippetAndTheWindowMayClose() async throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        open()
        let address = store.selection

        let inserted = await store.insertSelected()

        XCTAssertTrue(inserted)
        XCTAssertEqual(inserter.inserted, [address])
        XCTAssertNil(store.refusal)
    }

    func testEnterOnAnEmptyListInsertsNothing() async throws {
        open()

        let inserted = await store.insertSelected()

        XCTAssertFalse(inserted)
        XCTAssertTrue(inserter.inserted.isEmpty)
        // There is nothing to explain: the user picked nothing.
        XCTAssertNil(store.refusal)
    }

    func testARefusalKeepsThePaletteOpenAndSaysWhy() async throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        open()
        inserter.failure = .refused(.paused)

        let inserted = await store.insertSelected()

        XCTAssertFalse(inserted)
        let refusal = try XCTUnwrap(store.refusal)
        XCTAssertTrue(refusal.contains("paused"), refusal)
        // The palette is the one place a refusal is heard, so the window stays
        // up with the snippet still selected.
        XCTAssertEqual(store.selection, store.rows.first?.id)
    }

    func testEveryWayAPickCanFailHasWordsOfItsOwn() {
        let reasons: [InsertFailure] = [
            .refused(.paused), .refused(.excludedApp), .refused(.snippetGone), .noTargetApp
        ]
        let said = reasons.map(PaletteStore.describe)
        XCTAssertEqual(Set(said).count, reasons.count)
        XCTAssertTrue(said.allSatisfy { !$0.isEmpty })
    }

    func testAPickThatWorksClearsWhatTheLastOneSaid() async throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        open()
        inserter.failure = .refused(.excludedApp)
        await store.insertSelected()
        XCTAssertNotNil(store.refusal)

        inserter.failure = nil
        let inserted = await store.insertSelected()

        XCTAssertTrue(inserted)
        XCTAssertNil(store.refusal)
    }

    func testReopeningStartsOnAnEmptyQueryWithNothingLeftOver() async throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        try write("Standup", ";stand", body: "Yesterday:")
        open()
        store.query = "Standup"
        inserter.failure = .refused(.paused)
        await store.insertSelected()

        store.reset()

        XCTAssertEqual(store.query, "")
        XCTAssertNil(store.refusal)
        XCTAssertEqual(store.rows.count, 2)
    }

    func testAPaletteThatIsOpenSeesASnippetAddedUnderIt() throws {
        try write("Address", ";addr", body: "12 Mill Lane")
        open()
        try write("Standup", ";stand", body: "Yesterday:")

        store.refresh()

        XCTAssertEqual(Set(store.rows.map(\.label)), ["Address", "Standup"])
    }

    /// The index counts expansions on a thread of its own, so the recents list
    /// is a moment behind the expansion that made it.
    private func waitForRecents(toContain id: String, file: StaticString = #filePath, line: UInt = #line) throws {
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            if core.recents(limit: 10).contains(id) { return }
            RunLoop.current.run(until: Date().addingTimeInterval(0.02))
        }
        XCTFail("the index never counted the expansion", file: file, line: line)
    }
}

/// An inserter that writes down what it was asked for and answers however the
/// test needs it to.
@MainActor
final class RecordingInserter: SnippetInserter {
    private(set) var inserted: [String] = []
    var failure: InsertFailure?

    func insert(snippetId: String) async -> InsertFailure? {
        inserted.append(snippetId)
        return failure
    }
}
