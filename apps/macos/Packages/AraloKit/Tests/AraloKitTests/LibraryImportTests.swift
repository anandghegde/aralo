import AraloBridge
@testable import AraloKit
import XCTest

/// The import sheet's model, against the real core, a real folder and the
/// import corpus in `fixtures/import/`.
@MainActor
final class LibraryImportTests: XCTestCase {
    private var folder: URL!
    private var cache: URL!
    private var core: Core!
    private var store: LibraryStore!

    /// `fixtures/import/` at the root of the repository.
    private static let corpus = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent() // AraloKitTests
        .deletingLastPathComponent() // Tests
        .deletingLastPathComponent() // AraloKit
        .deletingLastPathComponent() // Packages
        .deletingLastPathComponent() // macos
        .deletingLastPathComponent() // apps
        .deletingLastPathComponent()
        .appendingPathComponent("fixtures/import")

    private static let work = corpus.appendingPathComponent("textexpander/work.textexpander")

    override func setUpWithError() throws {
        let run = UUID().uuidString
        let temporary = FileManager.default.temporaryDirectory
        folder = temporary.appendingPathComponent("aralo-\(run)")
        cache = temporary.appendingPathComponent("aralo-\(run)-cache")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try "format: 0\n".write(to: folder.appendingPathComponent("aralo.yaml"), atomically: true, encoding: .utf8)
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil, trash: nil)
        store = LibraryStore(core: core)
    }

    override func tearDownWithError() throws {
        store = nil
        core = nil
        for url in [folder, cache] { try? FileManager.default.removeItem(at: url!) }
    }

    private func files() -> [String] {
        let walker = FileManager.default.enumerator(atPath: folder.path)
        return (walker?.allObjects as? [String] ?? []).filter { $0.hasSuffix(".md") }
    }

    func testOpeningAnImportShowsWhatItWouldDoAndWritesNothing() throws {
        let job = store.importer(for: Self.work)
        let summary = try XCTUnwrap(job.summary, job.failure ?? "")
        XCTAssertTrue(summary.dryRun)
        XCTAssertEqual(summary.format, "textexpander")
        XCTAssertEqual(summary.total, 20)
        XCTAssertEqual(summary.imported, 19)
        XCTAssertEqual(job.headline, "19 of the 20 snippets in work.textexpander can be imported.")
        XCTAssertEqual(
            job.detail,
            "85% come over as they were; 2 snippets need an edit, though they expand; 1 snippet cannot come over."
        )
        XCTAssertTrue(job.canImport)
        XCTAssertEqual(files(), [])
        XCTAssertTrue(summary.entries.allSatisfy { $0.id == nil })
    }

    func testTheReportPutsWhatNeedsTheUserFirst() throws {
        let job = store.importer(for: Self.work)
        XCTAssertEqual(job.sections.map(\.outcome), [.needsEdit, .skipped, .clean])
        XCTAssertEqual(job.sections.map(\.entries.count), [2, 1, 17])
        // Every note says what the user has to do something about.
        XCTAssertTrue(job.sections[0].entries.allSatisfy { !$0.notes.isEmpty })
    }

    func testAGroupMovesEverythingUnderItAndTheDryRunFollows() throws {
        let job = store.importer(for: Self.work)
        job.group = " Imported / Work /"
        XCTAssertEqual(job.groupPath, ["Imported", "Work"])
        let entry = try XCTUnwrap(job.summary?.entries.first { $0.outcome != .skipped })
        XCTAssertEqual(Array(entry.group.prefix(2)), ["Imported", "Work"])
        XCTAssertTrue(entry.path?.hasPrefix("Imported/Work/") ?? false, entry.path ?? "no path")
        XCTAssertEqual(files(), [])
    }

    func testRunningItWritesTheSnippetsAndTheListShowsThem() throws {
        let job = store.importer(for: Self.work)
        job.group = "Imported"
        XCTAssertTrue(job.run())
        XCTAssertTrue(job.isDone)
        XCTAssertFalse(job.canImport)
        XCTAssertEqual(job.headline, "Imported 19 of 20 snippets from work.textexpander.")
        XCTAssertEqual(files().count, 19)
        // The window lands on what was imported.
        XCTAssertEqual(store.selectedGroup, ["Imported"])
        XCTAssertEqual(store.rows.count, 19)
        // A second run does nothing: the sheet is a report now.
        XCTAssertTrue(job.run())
        XCTAssertEqual(files().count, 19)
    }

    func testASnippetThatNeedsAnEditOpensFromTheReport() throws {
        let job = store.importer(for: Self.work)
        XCTAssertTrue(job.run())
        let entry = try XCTUnwrap(job.sections.first { $0.outcome == .needsEdit }?.entries.first)
        let id = try XCTUnwrap(entry.id)
        store.open(imported: entry)
        XCTAssertEqual(store.editing?.id, id)
        XCTAssertEqual(store.editing?.draft.label, entry.label)
    }

    func testChangingAnOptionRunsTheDryRunAgain() throws {
        let job = store.importer(for: Self.work)
        job.macros = .literal
        let summary = try XCTUnwrap(job.summary)
        XCTAssertTrue(summary.dryRun)
        // Kept as text, nothing is converted and nothing is lost to a macro
        // Aralo has no placeholder for, so nothing is worse off than before.
        XCTAssertGreaterThanOrEqual(summary.imported, 19)
        job.format = .csv
        // A property list read as a table is not the file it is.
        XCTAssertNotEqual(job.summary?.format, "textexpander")
    }

    func testAFileThatIsNoKnownFormatSaysSoAndOffersNothing() throws {
        let stray = folder.appendingPathComponent("notes.bin")
        try Data([0, 1, 2, 3]).write(to: stray)
        let job = store.importer(for: stray)
        XCTAssertNil(job.summary)
        XCTAssertNotNil(job.failure)
        XCTAssertFalse(job.canImport)
        XCTAssertFalse(job.run())
    }

    func testAnExportOfTheSelectedGroupImportsBack() throws {
        XCTAssertTrue(store.importer(for: Self.work).run())
        let json = try store.export(as: .json)
        let exported = folder.appendingPathComponent("exported.json")
        try json.write(to: exported)

        // Into a second library, which ends up with every snippet the first has.
        let other = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: other) }
        try FileManager.default.createDirectory(at: other, withIntermediateDirectories: true)
        try "format: 0\n".write(to: other.appendingPathComponent("aralo.yaml"), atomically: true, encoding: .utf8)
        let second = try Core.openLibrary(path: other.path, cache: other.path + "-cache", events: nil, trash: nil)
        let back = LibraryStore(core: second).importer(for: exported)
        XCTAssertEqual(back.summary?.total, 19)
        XCTAssertEqual(back.summary?.skipped, 0)
    }
}
