import AraloBridge
@testable import AraloKit
import XCTest

/// Copy Diagnostics, as the menu item builds it: the core's report from the
/// facts the shell passes. The Rust test drives every kind of content through
/// the bridge; this one checks the Swift end of the same promise.
final class DiagnosticsTests: XCTestCase {
    private var folder: URL!
    private var cache: URL!

    override func setUpWithError() throws {
        let run = UUID().uuidString
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-zqx-\(run)")
        cache = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-zqx-\(run)-cache")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try "format: 0\n".write(
            to: folder.appendingPathComponent("aralo.yaml"), atomically: true, encoding: .utf8
        )
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    func testTheReportCountsWhatHappenedAndHoldsNothingThatWasTyped() throws {
        let core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil, trash: nil)
        _ = try core.createSnippet(group: ["zqx-group"], draft: SnippetDraft(
            label: "zqx-label", abbreviations: [";zqx"], body: "zqx-body", tags: [], kind: .text,
            trigger: nil, case: nil, wholeWord: nil, keepDelimiter: nil, enabled: nil
        ))
        let engine = core.engine()
        engine.setFrontApp(bundleId: "com.apple.TextEdit")
        var expanded: String?
        for character in "zqx words ;zqx ".unicodeScalars {
            if case .expand(let id, _, _, _, _) = engine.onKey(key: .char(scalar: character.value)) {
                expanded = id
            }
        }
        let id = try XCTUnwrap(expanded)
        engine.expansionDone(snippetId: id, deleteCount: 8, method: .pasted)
        core.recordShellEvent(event: EventTap.Health.timedOut.shellEvent)
        core.recordShellEvent(event: .secureInputOn)

        let report = core.diagnosticsReport(
            facts: DiagnosticFacts(
                appVersion: "Aralo 0.5.0 (42)", osVersion: "macOS 15.1.0",
                accessibility: true, inputMonitoring: false, tapRunning: true,
                secureInput: false, paused: false, excludedApps: nil
            ),
            ai: nil
        )
        XCTAssertTrue(report.hasPrefix("Aralo diagnostics\n"), report)
        XCTAssertTrue(report.contains("  expansion.matched: 1\n"), report)
        XCTAssertTrue(report.contains("  insert.pasted: 1\n"), report)
        XCTAssertTrue(report.contains("  tap.timed_out: 1\n"), report)
        XCTAssertTrue(report.contains("  secure_input.on: 1\n"), report)
        XCTAssertTrue(report.contains("  input monitoring: not granted\n"), report)
        XCTAssertTrue(report.contains("  system: macOS "), report)
        XCTAssertFalse(report.lowercased().contains("zqx"), report)
    }
}
