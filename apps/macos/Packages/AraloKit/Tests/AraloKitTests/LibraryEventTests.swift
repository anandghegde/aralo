import AraloBridge
@testable import AraloKit
import XCTest

/// The core calling back into Swift, and the editing calls the main window
/// will be built on. `CoreEvents` is a foreign trait, so this direction is the
/// one the Rust tests cannot cover on their own.
final class LibraryEventTests: XCTestCase {
    /// Rust calls this from the thread watching the folder, not from this one.
    private final class Recorder: CoreEvents, @unchecked Sendable {
        private let lock = NSLock()
        private var events: [LibraryEvent] = []

        var heard: [LibraryEvent] {
            lock.lock()
            defer { lock.unlock() }
            return events
        }

        func libraryChanged(event: LibraryEvent) {
            lock.lock()
            defer { lock.unlock() }
            events.append(event)
        }
    }

    private var folder: URL!
    private var cache: URL!
    private var core: Core!
    private var recorder: Recorder!

    override func setUpWithError() throws {
        let run = UUID().uuidString
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)")
        cache = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)-cache")
        recorder = Recorder()
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: recorder, trash: nil)
    }

    override func tearDownWithError() throws {
        core = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    /// Polls rather than sleeps, so the test says what it is waiting for.
    private func waitUntil(_ what: String, _ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if condition() { return }
            RunLoop.current.run(until: Date().addingTimeInterval(0.01))
        }
        XCTFail("waited for \(what) and it never happened")
    }

    private func draft(_ label: String, _ abbreviation: String, _ body: String) -> SnippetDraft {
        SnippetDraft(
            label: label, abbreviations: [abbreviation], body: body, tags: [], kind: .text,
            trigger: nil, case: nil, wholeWord: nil, keepDelimiter: nil, enabled: nil
        )
    }

    func testASnippetTheShellWritesIsAFileAndComesBackAsAnEvent() throws {
        let id = try core.createSnippet(group: ["Work"], draft: draft("Address", ";addr", "1 Long Road"))
        let detail = try XCTUnwrap(core.snippet(id: id))
        XCTAssertEqual(detail.group, ["Work"])
        XCTAssertEqual(detail.preview, "1 Long Road")
        // Nothing was said about either, so both are inherited.
        XCTAssertNil(detail.draft.enabled)
        XCTAssertTrue(detail.resolved.enabled)
        XCTAssertTrue(FileManager.default.fileExists(atPath: folder.appendingPathComponent(detail.path).path))
        XCTAssertTrue(recorder.heard.contains(.edited))

        XCTAssertEqual(core.groups().first { $0.path == ["Work"] }?.snippets, 1)
        let hits = core.search(
            query: SearchQuery(text: "addr", group: nil, tag: nil, enabledOnly: false, limit: 5)
        )
        XCTAssertEqual(hits.first?.name, "Address")

        try core.deleteSnippet(id: id)
        XCTAssertNil(core.snippet(id: id))
    }

    func testAFileSomeoneElseWritesIsReadAndReported() throws {
        try "---\nabbr: ;hi\n---\nhello there".write(
            to: folder.appendingPathComponent("hello.md"), atomically: true, encoding: .utf8
        )
        waitUntil("the watch to read a file Aralo did not write") {
            recorder.heard.contains { event in
                if case .outside(let paths) = event { return paths.contains("hello.md") }
                return false
            }
        }
        XCTAssertTrue(core.snippets().contains { $0.abbreviations == [";hi"] })
    }

    func testTheEditorIsWarnedAboutAnAbbreviationThatIsTaken() throws {
        let taken = try core.createSnippet(group: [], draft: draft("Address", ";addr", "1 Long Road"))
        let problems = core.checkDraft(draft: draft("Other", ";addr", "elsewhere"), editing: nil)
        XCTAssertEqual(problems.count, 1)
        XCTAssertEqual(problems.first?.conflictsWith, taken)
        // The snippet on screen does not clash with itself.
        XCTAssertTrue(core.checkDraft(draft: draft("Address", ";addr", "1 Long Road"), editing: taken).isEmpty)
        XCTAssertEqual(core.suggestAbbreviation(label: "Thank You Very Much"), "tyvm")
    }
}
