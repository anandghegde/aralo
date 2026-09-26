import AraloBridge
@testable import AraloKit
import XCTest

/// The Library tab's end of moving the library (plan 5.2): what a picked
/// folder is taken to mean, and the move itself through the real core.
final class LibraryLocationTests: XCTestCase {
    private var home: URL!

    override func setUpWithError() throws {
        home = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-move-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: home)
    }

    @MainActor
    func testAFolderThatHoldsOtherFilesGetsAnAraloFolderInside() throws {
        let drive = home.appendingPathComponent("Library/Mobile Documents/com~apple~CloudDocs")
        try FileManager.default.createDirectory(at: drive, withIntermediateDirectories: true)
        try "notes".write(to: drive.appendingPathComponent("notes.txt"), atomically: true, encoding: .utf8)

        let target = LibraryLocationStore.destination(for: drive)
        XCTAssertEqual(target.lastPathComponent, "Aralo")
        let location = inspectLibraryLocation(path: target.path)
        XCTAssertEqual(location.provider, "iCloud Drive")
        XCTAssertEqual(location.contents, .empty)

        let empty = home.appendingPathComponent("Empty")
        try FileManager.default.createDirectory(at: empty, withIntermediateDirectories: true)
        XCTAssertEqual(LibraryLocationStore.destination(for: empty), empty)
    }

    func testTheLibraryMovesAndExpandsFromTheNewFolder() throws {
        let from = home.appendingPathComponent("Aralo")
        let destination = home.appendingPathComponent("Dropbox/Aralo")
        let core = try Core.openLibrary(
            path: from.path, cache: home.appendingPathComponent("cache").path, events: nil, trash: nil
        )
        let moved = try core.moveLibrary(to: destination.path)
        XCTAssertEqual(moved.changedAfterCopy, [])
        XCTAssertEqual(core.libraryPath(), moved.to)
        XCTAssertTrue(FileManager.default.fileExists(atPath: from.path), "the old folder stays until trashed")

        let engine = core.engine()
        engine.setFrontApp(bundleId: "com.apple.TextEdit")
        var expanded = false
        for character in "ty ".unicodeScalars {
            if case .expand = engine.onKey(key: .char(scalar: character.value)) { expanded = true }
        }
        XCTAssertTrue(expanded)
    }
}
