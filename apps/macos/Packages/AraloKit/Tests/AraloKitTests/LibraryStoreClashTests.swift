import AraloBridge
@testable import AraloKit
import XCTest

/// Saving a draft whose file changed on disk while it was open (plan 5.1),
/// against the real core and a real folder: a change elsewhere in the file is
/// merged in, and one to what the draft changed stops the window until the
/// user picks.
@MainActor
final class LibraryStoreClashTests: XCTestCase {
    private var folder: URL!
    private var cache: URL!
    private var core: Core!
    private var store: LibraryStore!

    override func setUpWithError() throws {
        let run = UUID().uuidString
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)")
        cache = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)-cache")
        // A folder that is already a library gets no starter snippets, so every
        // count below is of what the test wrote and nothing else.
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try "format: 0\n".write(
            to: folder.appendingPathComponent("aralo.yaml"), atomically: true, encoding: .utf8
        )
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil, trash: nil)
        store = LibraryStore(core: core)
    }

    override func tearDownWithError() throws {
        store = nil
        core = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    private func draft(_ label: String, _ abbreviation: String, _ body: String) -> SnippetDraft {
        SnippetDraft(
            label: label, abbreviations: [abbreviation], body: body, tags: [], kind: .text,
            trigger: nil, case: nil, wholeWord: nil, keepDelimiter: nil, enabled: nil
        )
    }

    private func write(_ label: String, _ abbreviation: String, into group: [String]) throws -> String {
        let id = try core.createSnippet(group: group, draft: draft(label, abbreviation, "body of \(label)"))
        store.refresh()
        return id
    }

    /// The file as it sits on disk, which is the thing the user's Git history
    /// will show.
    private func fileText(_ path: String) throws -> String {
        try String(contentsOf: folder.appendingPathComponent(path), encoding: .utf8)
    }

    /// Another Mac's edit, as a sync client writes it: the file replaced,
    /// with the folder not read again yet.
    private func otherMacWrites(_ id: String, label: String, body: String) throws {
        let path = try XCTUnwrap(core.snippet(id: id)).path
        try "---\nid: \(id)\nlabel: \(label)\nabbr: \";stand\"\n---\n\(body)".write(
            to: folder.appendingPathComponent(path), atomically: true, encoding: .utf8
        )
    }

    private func type(_ body: String) throws {
        var editing = try XCTUnwrap(store.editing)
        editing.draft.body = body
        store.editing = editing
    }

    func testASaveKeepsWhatAnotherMacChangedMeanwhile() throws {
        let id = try write("Standup", ";stand", into: [])
        store.select(snippet: id)
        try type("Yesterday:\nToday:\nBlockers:")
        try otherMacWrites(id, label: "Daily standup", body: "body of Standup")

        XCTAssertTrue(store.leave())
        XCTAssertNil(store.clash)
        let text = try fileText(try XCTUnwrap(store.editing).path)
        XCTAssertTrue(text.contains("label: Daily standup"), text)
        XCTAssertTrue(text.contains("Blockers:"), text)
        XCTAssertFalse(try XCTUnwrap(store.editing).isDirty)
    }

    func testAClashWritesNothingAndKeepsTheWindowOnTheDraft() throws {
        let id = try write("Standup", ";stand", into: [])
        store.select(snippet: id)
        let path = try XCTUnwrap(store.editing).path
        try type("mine")
        try otherMacWrites(id, label: "Standup", body: "theirs")
        let onDisk = try fileText(path)

        XCTAssertFalse(store.leave(), "a clash keeps the window where it is")
        let clash = try XCTUnwrap(store.clash)
        XCTAssertEqual(clash.id, id)
        XCTAssertEqual(clash.summary, "Both changed the text, differently.")
        XCTAssertTrue(clash.mineText.contains("mine"), clash.mineText)
        XCTAssertEqual(clash.diskText, onDisk)
        XCTAssertEqual(try fileText(path), onDisk, "nothing was written")
        XCTAssertEqual(store.editing?.draft.body, "mine")

        // Put away undecided, the next save finds it again.
        store.dismissClash()
        XCTAssertFalse(store.leave())

        try store.keepMine()
        XCTAssertNil(store.clash)
        XCTAssertTrue(try fileText(path).contains("mine"))
        XCTAssertFalse(try XCTUnwrap(store.editing).isDirty)
    }

    func testUsingTheFileOnDiskDropsTheDraft() throws {
        let id = try write("Standup", ";stand", into: [])
        store.select(snippet: id)
        try type("mine")
        try otherMacWrites(id, label: "Standup", body: "theirs")
        XCTAssertFalse(store.leave())

        try store.useTheirs()
        XCTAssertNil(store.clash)
        XCTAssertEqual(store.editing?.draft.body, "theirs")
        XCTAssertFalse(try XCTUnwrap(store.editing).isDirty)
    }

    func testEditingFirstSavesOverTheFileAsItNowIs() throws {
        let id = try write("Standup", ";stand", into: [])
        store.select(snippet: id)
        let path = try XCTUnwrap(store.editing).path
        try type("mine")
        try otherMacWrites(id, label: "Standup", body: "theirs")
        XCTAssertFalse(store.leave())

        store.editBeforeSaving()
        XCTAssertNil(store.clash)
        XCTAssertEqual(store.editing?.draft.body, "mine")
        XCTAssertTrue(try XCTUnwrap(store.editing).isDirty)
        XCTAssertTrue(try fileText(path).contains("theirs"), "nothing was written")

        try type("mine, and theirs")
        XCTAssertTrue(store.leave())
        XCTAssertTrue(try fileText(path).contains("mine, and theirs"))
    }
}
