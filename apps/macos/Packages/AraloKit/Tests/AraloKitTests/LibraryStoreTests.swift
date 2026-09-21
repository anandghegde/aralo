import AraloBridge
@testable import AraloKit
import XCTest

/// The main window's model, against the real core and a real folder. Every
/// assertion here is about what the three panes show and what a change to the
/// folder leaves behind.
@MainActor
final class LibraryStoreTests: XCTestCase {
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
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil)
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

    @discardableResult
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

    // MARK: - The sidebar

    func testTheTreeNestsGroupsAndCountsWhatIsUnderThem() throws {
        try write("Standup", ";stand", into: ["Work"])
        try write("Nit", ";nit", into: ["Work", "Reviews"])
        try write("Address", ";addr", into: [])

        let root = try XCTUnwrap(store.root)
        XCTAssertTrue(root.isRoot)
        // One snippet sits in the root itself; the other two are below it.
        XCTAssertEqual(root.snippets, 1)
        XCTAssertEqual(root.total, 3)

        let work = try XCTUnwrap(root.find(["Work"]))
        XCTAssertEqual(work.name, "Work")
        XCTAssertEqual(work.snippets, 1)
        XCTAssertEqual(work.total, 2)
        XCTAssertEqual(work.children.map(\.name), ["Reviews"])
        XCTAssertEqual(root.flattened.map(\.id), ["", "Work", "Work/Reviews"])
    }

    func testSelectingAGroupListsWhatIsInsideItAndSearchNarrowsThat() throws {
        try write("Standup", ";stand", into: ["Work"])
        try write("Nit", ";nit", into: ["Work", "Reviews"])
        try write("Address", ";addr", into: ["Home"])

        // The root is the whole library, which is what the window opens on.
        XCTAssertEqual(store.rows.count, 3)

        store.select(group: ["Work"])
        XCTAssertEqual(Set(store.rows.map(\.label)), ["Standup", "Nit"])

        // The search box narrows the selected group rather than replacing it.
        store.search("addr")
        XCTAssertTrue(store.rows.isEmpty)
        store.select(group: [])
        XCTAssertEqual(store.rows.map(\.label), ["Address"])
        XCTAssertFalse(store.rows[0].matched.isEmpty, "a hit carries what matched, for the row to highlight")

        // An empty search is the list again, with nothing highlighted.
        store.search("")
        XCTAssertEqual(store.rows.count, 3)
        XCTAssertTrue(store.rows.allSatisfy { $0.matched.isEmpty })
    }

    // MARK: - Editing a snippet

    func testAnEditLandsAsAReadableFileDiff() throws {
        let id = try write("Standup", ";stand", into: ["Work"])
        store.select(snippet: id)
        var editing = try XCTUnwrap(store.editing)
        XCTAssertFalse(editing.isDirty)
        XCTAssertEqual(editing.preview, "body of Standup")

        editing.draft.label = "Daily Standup"
        editing.draft.abbreviations = [";stand", ";su"]
        editing.draft.tags = ["work", "ritual"]
        editing.draft.body = "Yesterday I ...\nToday I ..."
        store.editing = editing
        XCTAssertTrue(try XCTUnwrap(store.editing).isDirty)
        try store.save()

        XCTAssertFalse(try XCTUnwrap(store.editing).isDirty)
        let text = try fileText(try XCTUnwrap(store.editing).path)
        // The file is the record: front matter a person reads, then the body.
        XCTAssertTrue(text.contains("label: Daily Standup"), text)
        XCTAssertTrue(text.contains(";su"), text)
        XCTAssertTrue(text.contains("work"), text)
        let body = text.drop { $0 != "\n" }.drop(while: \.isNewline)
        XCTAssertTrue(
            text.trimmingCharacters(in: .newlines).hasSuffix("Yesterday I ...\nToday I ..."),
            "the body is the end of the file, not a quoted field: \(body)"
        )
        // The ID survived the edit, so the list did not lose its selection.
        XCTAssertEqual(store.editing?.id, id)
    }

    func testANewSnippetIsWrittenIntoTheSelectedGroupWithAFreeAbbreviation() throws {
        try write("Thank You", ";ty", into: [])
        // A new group is selected as it is made, so the snippet lands in it.
        try store.newGroup(named: "Work", under: [])
        XCTAssertEqual(store.selectedGroup, ["Work"])

        let id = try store.newSnippet(label: "Thank You")
        let editing = try XCTUnwrap(store.editing)
        XCTAssertEqual(editing.id, id)
        XCTAssertEqual(editing.group, ["Work"])
        // ";ty" is taken, so the suggestion is not it, and nothing is reported.
        XCTAssertNotEqual(editing.draft.abbreviations, [";ty"])
        XCTAssertTrue(store.problems.isEmpty, "\(store.problems)")
        XCTAssertTrue(store.rows.contains { $0.id == id })
    }

    func testATakenAbbreviationIsReportedAndStillSaves() throws {
        let taken = try write("Address", ";addr", into: [])
        let id = try write("Other", ";other", into: [])
        store.select(snippet: id)
        var editing = try XCTUnwrap(store.editing)
        editing.draft.abbreviations = [";addr"]
        store.editing = editing

        XCTAssertEqual(store.problems.count, 1)
        XCTAssertEqual(store.problems.first?.conflictsWith, taken)

        // Advice, not a refusal: the folder is the user's.
        try store.save()
        XCTAssertEqual(store.editing?.draft.abbreviations, [";addr"])
        XCTAssertEqual(store.problems.count, 1, "the clash is still true after the save")
    }

    func testRevertGoesBackToWhatTheFileSays() throws {
        let id = try write("Standup", ";stand", into: [])
        store.select(snippet: id)
        var editing = try XCTUnwrap(store.editing)
        editing.draft.body = "typed and thought better of"
        store.editing = editing

        store.revert()
        XCTAssertEqual(store.editing?.draft.body, "body of Standup")
        XCTAssertFalse(try XCTUnwrap(store.editing).isDirty)
    }

    func testDeletingTheOpenSnippetClosesTheEditor() throws {
        let id = try write("Standup", ";stand", into: [])
        store.select(snippet: id)
        try store.delete(snippet: id)
        XCTAssertNil(store.editing)
        XCTAssertTrue(store.rows.isEmpty)
    }

    // MARK: - Editing groups

    func testRenamingAGroupTakesTheSelectionAndTheOpenSnippetWithIt() throws {
        let id = try write("Standup", ";stand", into: ["Work", "Reviews"])
        store.select(group: ["Work", "Reviews"])
        store.select(snippet: id)

        let moved = try store.rename(group: ["Work"], to: "Job")
        XCTAssertEqual(moved, ["Job"])
        XCTAssertEqual(store.selectedGroup, ["Job", "Reviews"])
        XCTAssertEqual(store.rows.map(\.label), ["Standup"])
        // The ID follows the file, so the editor is still on the same snippet.
        XCTAssertEqual(store.editing?.id, id)
        XCTAssertEqual(store.editing?.group, ["Job", "Reviews"])
    }

    func testMovingASnippetChangesItsFolderAndKeepsItsID() throws {
        let id = try write("Standup", ";stand", into: ["Work"])
        try store.newGroup(named: "Archive", under: [])
        store.select(snippet: id)

        try store.move(snippet: id, to: ["Archive"])
        XCTAssertEqual(store.editing?.id, id)
        XCTAssertEqual(store.editing?.group, ["Archive"])
        XCTAssertTrue(try XCTUnwrap(store.editing).path.hasPrefix("Archive/"))
    }

    func testANewGroupNeverOverwritesOneThatIsThere() throws {
        let first = try store.newGroup(named: "Work", under: [])
        let second = try store.newGroup(named: "Work", under: [])
        XCTAssertEqual(first, ["Work"])
        XCTAssertEqual(second, ["Work 2"])
        XCTAssertEqual(store.root?.children.map(\.name), ["Work", "Work 2"])
    }

    func testSwitchingAGroupOffTakesEverythingInsideItWithIt() throws {
        try write("Standup", ";stand", into: ["Work"])
        try write("Nit", ";nit", into: ["Work", "Reviews"])

        try store.setEnabled(false, group: ["Work"])
        let work = try XCTUnwrap(store.root?.find(["Work"]))
        XCTAssertFalse(work.enabled)
        // Off is sticky: the group below it says nothing and is off too.
        XCTAssertFalse(try XCTUnwrap(store.root?.find(["Work", "Reviews"])).enabled)
        XCTAssertTrue(store.rows.allSatisfy { !$0.enabled })
        XCTAssertTrue(try fileText("Work/_group.yaml").contains("enabled: false"))
    }

    func testDeletingAGroupSaysWhatWentAndClearsTheSelection() throws {
        try write("Standup", ";stand", into: ["Work"])
        let id = try write("Nit", ";nit", into: ["Work", "Reviews"])
        store.select(group: ["Work", "Reviews"])
        store.select(snippet: id)
        XCTAssertEqual(store.contents(of: ["Work"]), 2)

        let removed = try store.delete(group: ["Work"])
        XCTAssertEqual(removed.count, 2, "\(removed)")
        XCTAssertEqual(store.selectedGroup, [])
        XCTAssertNil(store.editing)
        XCTAssertNil(store.root?.find(["Work"]))
    }

    func testAGroupsColourAndIconAreWrittenAndComeBack() throws {
        try store.newGroup(named: "Work", under: [])
        try store.setAppearance(group: ["Work"], colour: "#FF8800", icon: "briefcase")
        let work = try XCTUnwrap(store.root?.find(["Work"]))
        XCTAssertEqual(work.colour, "#FF8800")
        XCTAssertEqual(work.icon, "briefcase")
    }

    // MARK: - Changes from outside

    func testAChangeUnderneathNeverOverwritesWhatHasBeenTyped() throws {
        let id = try write("Standup", ";stand", into: [])
        store.select(snippet: id)
        var editing = try XCTUnwrap(store.editing)
        editing.draft.body = "half-typed"
        store.editing = editing

        // Someone adds a snippet in a text editor and the watch reports it.
        try core.createSnippet(group: [], draft: draft("Address", ";addr", "1 Long Road"))
        store.libraryChanged(.outside(paths: ["address.md"]))

        XCTAssertEqual(store.rows.count, 2, "the list took the new file")
        XCTAssertEqual(store.editing?.draft.body, "half-typed", "the draft on screen is the user's work")
        XCTAssertTrue(try XCTUnwrap(store.editing).isDirty)
    }

    func testAnUneditedSnippetIsReReadWhenItsFileChanges() throws {
        let id = try write("Standup", ";stand", into: [])
        store.select(snippet: id)
        let path = try XCTUnwrap(store.editing).path

        try "---\nid: \(id)\nlabel: Standup\nabbr: \";stand\"\n---\nrewritten by hand".write(
            to: folder.appendingPathComponent(path), atomically: true, encoding: .utf8
        )
        try core.reload()
        store.libraryChanged(.reloaded)

        XCTAssertEqual(store.editing?.draft.body, "rewritten by hand")
        XCTAssertFalse(try XCTUnwrap(store.editing).isDirty)
    }

    func testAFailedCommandIsKeptInTheCoresOwnWords() throws {
        try write("Standup", ";stand", into: [])
        store.attempt { try $0.rename(group: ["Nowhere"], to: "Somewhere") }

        let failure = try XCTUnwrap(store.failure)
        XCTAssertFalse(
            failure.contains("BridgeError"),
            "the window shows the sentence the core wrote, not the Swift case spelled out: \(failure)"
        )
        XCTAssertFalse(failure.isEmpty)
        XCTAssertEqual(store.rows.count, 1, "nothing was lost by the call that failed")

        store.attempt { try $0.newGroup(named: "Work", under: []) }
        XCTAssertNil(store.failure, "the next call that works clears it")
    }

    func testThePreviewFollowsWhatIsBeingTypedRatherThanTheFile() throws {
        let id = try write("Sign off", ";sig", into: [])
        store.select(snippet: id)
        store.editing?.draft.body = "Kind regards,\nA"

        XCTAssertEqual(store.preview(of: try XCTUnwrap(store.editing).draft.body), "Kind regards,\nA")
        XCTAssertEqual(store.editing?.preview, "body of Sign off", "the file has not changed yet")
    }

    func testAGroupWithNothingInsideItHasNoSubgroups() throws {
        try core.createGroup(group: ["Work"])
        try core.createGroup(group: ["Work", "Standup"])
        store.refresh()

        let root = try XCTUnwrap(store.root)
        XCTAssertEqual(try XCTUnwrap(root.find(["Work"])?.subgroups)?.count, 1)
        XCTAssertNil(root.find(["Work", "Standup"])?.subgroups, "a leaf draws no disclosure triangle")
    }

    func testAFailedReadIsShownAndTheLibraryStaysUp() throws {
        try write("Standup", ";stand", into: [])
        store.libraryChanged(.failed(message: "the folder went away"))
        XCTAssertEqual(store.problem, "the folder went away")
        XCTAssertEqual(store.rows.count, 1, "the library that loaded is still the one in use")

        store.libraryChanged(.reloaded)
        XCTAssertNil(store.problem)
    }
}
