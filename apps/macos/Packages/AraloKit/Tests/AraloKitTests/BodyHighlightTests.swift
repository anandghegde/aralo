import AraloBridge
@testable import AraloKit
import XCTest

/// What the body editor draws, against the real core: the ranges come from the
/// same reading of the body that an expansion does, so what is underlined here
/// is what would happen (PRD L10).
@MainActor
final class BodyHighlightTests: XCTestCase {
    private var folder: URL!
    private var cache: URL!
    private var core: Core!
    private var store: LibraryStore!

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
    }

    override func tearDownWithError() throws {
        store = nil
        core = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    // MARK: - The core's reading, through the store

    /// M2's done-when: an unclosed `{{` is underlined at the right range, and
    /// the range is the one the rest of the body occupies, because that is what
    /// expands as literal text.
    func testAnUnclosedOpeningIsUnderlinedOverTheRestOfTheBody() {
        let body = "Hi {{date: %Y"
        let highlight = store.outline(of: body)
        XCTAssertEqual(
            highlight.runs,
            [BodyRun(range: NSRange(location: 3, length: 10), style: .error)]
        )
        XCTAssertEqual(highlight.advice.count, 1)
        XCTAssertTrue(highlight.advice[0].isError)
        XCTAssertEqual(highlight.advice[0].range, NSRange(location: 3, length: 10))
        XCTAssertFalse(highlight.advice[0].message.isEmpty)
        XCTAssertEqual((body as NSString).substring(with: highlight.runs[0].range), "{{date: %Y")
    }

    func testABodyThatReadsCleanlyHasNothingToSay() {
        let highlight = store.outline(of: "Thank you for the note.")
        XCTAssertTrue(highlight.isEmpty)
    }

    func testEveryPlaceholderIsColouredAndAnUnknownOneIsPointedAt() {
        let body = "{{clipboard}} then {{shrubbery}}"
        let highlight = store.outline(of: body)
        XCTAssertEqual(
            highlight.runs,
            [
                BodyRun(range: NSRange(location: 0, length: 13), style: .placeholder),
                BodyRun(range: NSRange(location: 19, length: 13), style: .unknownPlaceholder),
                BodyRun(range: NSRange(location: 19, length: 13), style: .note)
            ]
        )
        XCTAssertEqual(highlight.advice.count, 1)
        XCTAssertFalse(highlight.advice[0].isError)
        XCTAssertTrue(highlight.advice[0].message.contains("shrubbery"))
    }

    /// The offsets are UTF-16 code units, not characters and not bytes: an
    /// emoji ahead of a placeholder must not shift the underline.
    func testRangesAreCountedTheWayATextViewCountsThem() throws {
        let body = "👋 {{cursor}}"
        let highlight = store.outline(of: body)
        let range = try XCTUnwrap(highlight.runs.first?.range)
        XCTAssertEqual(range, NSRange(location: 3, length: 10))
        XCTAssertEqual((body as NSString).substring(with: range), "{{cursor}}")
    }

    func testAnEscapedOpeningIsNotAPlaceholder() {
        XCTAssertTrue(store.outline(of: "\\{{not one}}").isEmpty)
    }

    // MARK: - Keeping the ranges inside the text

    /// The outline is read on one keystroke and drawn on the next, so it can
    /// describe a body longer than the one on screen. What is left of a range
    /// is drawn; what has gone is not, and nothing throws.
    func testARangeTheBodyHasGrownPastIsCutBackOrDropped() {
        let outline = BodyOutline(
            placeholders: [
                BodyPlaceholder(name: "cursor", known: true, evaluated: false, start: 2, end: 12),
                BodyPlaceholder(name: "date", known: true, evaluated: false, start: 40, end: 50)
            ],
            problems: []
        )
        let highlight = BodyHighlight(outline: outline, body: "ab{{cur")
        XCTAssertEqual(
            highlight.runs,
            [BodyRun(range: NSRange(location: 2, length: 5), style: .placeholder)]
        )
    }

    /// A message is never dropped with its range: the reader is told what is
    /// wrong even when there is nothing left to underline.
    func testAProblemOutsideTheBodyStillSpeaks() {
        let outline = BodyOutline(
            placeholders: [],
            problems: [
                BodyProblem(level: .error, message: "This {{ has no }} after it.", start: 30, end: 44)
            ]
        )
        let highlight = BodyHighlight(outline: outline, body: "short")
        XCTAssertTrue(highlight.runs.isEmpty)
        XCTAssertEqual(highlight.advice.count, 1)
        XCTAssertEqual(highlight.advice[0].range, NSRange(location: 5, length: 0))
    }

    /// Problems are drawn after the placeholders they are about, so the
    /// underline is not painted over by the colour underneath it.
    func testProblemsAreDrawnOverThePlaceholdersTheyAreAbout() {
        let outline = BodyOutline(
            placeholders: [BodyPlaceholder(name: "date", known: true, evaluated: false, start: 0, end: 20)],
            problems: [BodyProblem(level: .error, message: "An option is written key: value.", start: 8, end: 19)]
        )
        let highlight = BodyHighlight(outline: outline, body: String(repeating: "x", count: 20))
        XCTAssertEqual(highlight.runs.map(\.style), [.placeholder, .error])
    }

    // MARK: - The insert menu

    func testTheInsertMenuOffersThePlaceholdersTheCoreKnows() {
        let names = Placeholder.all.map(\.name)
        XCTAssertTrue(names.contains("date"))
        XCTAssertTrue(names.contains("cursor"))
        XCTAssertEqual(names.count, Set(names).count)

        let expands = Dictionary(uniqueKeysWithValues: Placeholder.all.map { ($0.name, $0.expands) })
        XCTAssertEqual(expands["date"], true)
        XCTAssertEqual(expands["clipboard"], true)
        XCTAssertEqual(expands["field"], true)
        XCTAssertEqual(expands["ai"], true)
        // Waiting on a later milestone: the core inserts this as the text it is
        // written as, and a menu can say so instead of promising a value.
        XCTAssertEqual(expands["selection"], false)
    }

    /// What the menu inserts reads back as one placeholder of that name, and
    /// the part it pre-selects is the reader's to type over: never a brace.
    func testWhatTheMenuInsertsReadsBackAsThatPlaceholder() throws {
        for choice in Placeholder.all {
            var body = "Dear reader, "
            let caret = body.utf16.count
            body += choice.insert
            let highlight = store.outline(of: body)
            // A note is allowed: a placeholder the core does not expand yet is
            // still one it knows, and the outline says so rather than
            // pretending. An error would mean the menu offered a body the
            // editor calls wrong. `{{snippet: …}}` is the exception: its sample
            // names a snippet this library does not have, and drawing that is
            // the editor doing its job.
            if choice.name != "snippet" {
                XCTAssertTrue(
                    highlight.advice.allSatisfy { !$0.isError },
                    "\(choice.name) does not read back cleanly: \(highlight.advice)"
                )
            }
            let inserted = NSRange(location: caret, length: choice.insert.utf16.count)
            XCTAssertEqual(
                highlight.runs.first,
                BodyRun(range: inserted, style: .placeholder),
                "\(choice.name) is not one known placeholder"
            )
            // Whatever else is drawn is drawn over that same placeholder: the
            // advice is about what the menu inserted, not about the sentence in
            // front of it.
            XCTAssertTrue(
                highlight.runs.dropFirst().allSatisfy { $0.range == inserted },
                "\(choice.name) draws outside what it inserted: \(highlight.runs)"
            )
            let selection = choice.selection(insertedAt: caret)
            XCTAssertGreaterThanOrEqual(selection.location, caret)
            XCTAssertLessThanOrEqual(selection.upperBound, body.utf16.count)
            let selected = (body as NSString).substring(with: selection)
            XCTAssertFalse(selected.contains("{"), "\(choice.name) selects syntax")
            XCTAssertFalse(selected.contains("}"), "\(choice.name) selects syntax")
            XCTAssertFalse(selected.contains("|"), "\(choice.name) selects syntax")
            XCTAssertEqual(selected, selected.trimmingCharacters(in: .whitespaces))
        }
    }
}
