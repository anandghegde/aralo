import AraloBridge
@testable import AraloKit
import XCTest

/// Task 5.5 across the bridge: the signed compatibility table download as the
/// app sees it. The checks themselves are tested in Rust
/// (`aralo-core/tests/data_update.rs`).
final class DataTablesTests: XCTestCase {
    private var folder: URL!
    private var cache: URL!
    private var core: Core!

    override func setUpWithError() throws {
        let run = UUID().uuidString
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)")
        cache = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-\(run)-cache")
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil, trash: nil)
    }

    override func tearDownWithError() throws {
        core = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    /// A build without a real data-table key asks nothing and keeps the
    /// bundled table.
    func testWithoutADataTableKeyARefreshAsksNothing() async throws {
        let before = core.compatTableStatus()
        XCTAssertEqual(before.source, .bundled)
        XCTAssertNil(before.lastRefresh)
        guard !before.hasKey else { return }
        let settings = try AiProfiles.open(path: cache.appendingPathComponent("profiles.toml").path, keys: .memory)
        let outcome = await core.refreshCompatTable(ai: settings)
        guard case .notChecked = outcome else { return XCTFail("\(outcome)") }
        let after = core.compatTableStatus()
        XCTAssertEqual(after.source, .bundled)
        XCTAssertEqual(after.revision, before.revision)
        XCTAssertEqual(after.lastRefresh, outcome)
        XCTAssertFalse(FileManager.default.fileExists(atPath: cache.appendingPathComponent("data-tables").path))
    }
}
