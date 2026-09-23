import AraloBridge
@testable import AraloKit
import XCTest

/// The resolver's model, against the real core, a real folder, and a Trash
/// that is a folder of the test's own: the real one is not a test's to fill.
@MainActor
final class ConflictResolverTests: XCTestCase {
    private var folder: URL!
    private var cache: URL!
    private var bin: URL!
    private var core: Core!
    private var store: LibraryStore!

    private static let id = "01J8ZK3V5Q8W6T9X2N4R7M0ABC"

    /// Moves what it is given into `folder`.
    private final class FolderTrash: Trash, @unchecked Sendable {
        let folder: URL
        init(folder: URL) { self.folder = folder }

        func discard(path: String) throws {
            let from = URL(fileURLWithPath: path)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try FileManager.default.moveItem(at: from, to: folder.appendingPathComponent(from.lastPathComponent))
        }
    }

    private static func snippet(_ label: String, _ body: String = "Best,\nSam") -> String {
        "---\nid: \(Self.id)\nlabel: \(label)\nabbr: [;sig]\n---\n\(body)\n"
    }

    override func setUpWithError() throws {
        let run = UUID().uuidString
        let temporary = FileManager.default.temporaryDirectory
        folder = temporary.appendingPathComponent("aralo-\(run)")
        cache = temporary.appendingPathComponent("aralo-\(run)-cache")
        bin = temporary.appendingPathComponent("aralo-\(run)-trash")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try "format: 0\n".write(to: folder.appendingPathComponent("aralo.yaml"), atomically: true, encoding: .utf8)
        // Both machines renamed the signature, and one changed its text too.
        try Self.snippet("Mine").write(to: folder.appendingPathComponent("sig.md"), atomically: true, encoding: .utf8)
        let copy = folder.appendingPathComponent("sig (conflicted copy 2026-09-23).md")
        try Self.snippet("Theirs", "Best wishes,\nSam").write(to: copy, atomically: true, encoding: .utf8)
        core = try Core.openLibrary(
            path: folder.path, cache: cache.path, events: nil, trash: FolderTrash(folder: bin)
        )
        store = LibraryStore(core: core)
    }

    override func tearDownWithError() throws {
        store = nil
        core = nil
        for url in [folder, cache, bin] { try? FileManager.default.removeItem(at: url!) }
    }

    private func files() throws -> [String] {
        try FileManager.default.contentsOfDirectory(atPath: folder.path).filter { $0.hasSuffix(".md") }.sorted()
    }

    func testTheStoreListsTheCopyAgainstItsSnippet() throws {
        XCTAssertEqual(store.conflicts.map(\.copy), ["sig (conflicted copy 2026-09-23).md"])
        XCTAssertEqual(store.conflict(for: Self.id)?.original, "sig.md")
        XCTAssertNil(store.conflict(for: "01J8ZK3V5Q8W6T9X2N4R7M0XYZ"))
    }

    func testTheResolverSaysWhatBothSidesChanged() throws {
        let resolver = try XCTUnwrap(store.resolver(for: "sig (conflicted copy 2026-09-23).md"))
        XCTAssertEqual(resolver.originalName, "sig.md")
        XCTAssertEqual(resolver.copyName, "sig (conflicted copy 2026-09-23).md")
        XCTAssertTrue(resolver.copyIsReadable)
        // There is no earlier version yet, so every difference is a clash.
        XCTAssertEqual(resolver.summary, "Both versions changed the name and the text, differently.")
        XCTAssertEqual(resolver.edited, Self.snippet("Mine"))
    }

    func testKeepingTheCopyPutsItInTheOriginalAndTheCopyInTheTrash() throws {
        let resolver = try XCTUnwrap(store.resolver(for: "sig (conflicted copy 2026-09-23).md"))
        XCTAssertTrue(resolver.keepCopy())
        XCTAssertEqual(try files(), ["sig.md"])
        XCTAssertTrue(FileManager.default.fileExists(
            atPath: bin.appendingPathComponent("sig (conflicted copy 2026-09-23).md").path
        ))
        XCTAssertTrue(store.conflicts.isEmpty)
        XCTAssertEqual(core.snippet(id: Self.id)?.draft.label, "Theirs")
    }

    func testAnEditedVersionIsKeptAndABrokenOneIsRefused() throws {
        let resolver = try XCTUnwrap(store.resolver(for: "sig (conflicted copy 2026-09-23).md"))
        resolver.edited = "---\nabbr: [\n---\n"
        XCTAssertFalse(resolver.keepEdited())
        XCTAssertNotNil(resolver.failure)
        XCTAssertEqual(store.conflicts.count, 1)

        resolver.edited = Self.snippet("Both", "Best wishes,\nSam")
        XCTAssertTrue(resolver.keepEdited())
        XCTAssertNil(resolver.failure)
        XCTAssertEqual(try files(), ["sig.md"])
        XCTAssertEqual(core.snippet(id: Self.id)?.draft.label, "Both")
    }

    func testACopyThatHasGoneHasNoResolver() {
        XCTAssertNil(store.resolver(for: "nothing.md"))
        XCTAssertNotNil(store.failure)
    }

    func testKeysAreNamedAsTheWindowNamesThem() {
        XCTAssertEqual(ConflictResolver.describe("abbr"), "the abbreviations")
        XCTAssertEqual(ConflictResolver.describe("future"), "\u{201C}future\u{201D}")
        XCTAssertEqual(ConflictResolver.list(["a", "b", "c"]), "a, b and c")
    }
}
