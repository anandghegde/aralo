@testable import AraloHarness
import XCTest

final class ReportTests: XCTestCase {
    func testPercentilesAreNearestRank() {
        let samples = (1...100).map(Double.init)
        XCTAssertEqual(Percentile.of(samples, 50), 50)
        XCTAssertEqual(Percentile.of(samples, 95), 95)
        XCTAssertEqual(Percentile.of(samples, 100), 100)
        XCTAssertEqual(Percentile.of([7], 95), 7)
        XCTAssertEqual(Percentile.of([9, 1, 5], 50), 5)
        XCTAssertNil(Percentile.of([], 95))
    }

    private var report: Report {
        Report(config: "chosen", apps: [
            AppResult(name: "TextEdit", bundleID: "com.apple.TextEdit", cases: [
                CaseResult(.ascii, .pass, attempts: 1, method: "type", readBack: .accessibility, latencyMs: 12),
                CaseResult(.long, .flaky, attempts: 2, method: "paste"),
                CaseResult(.cursor, .pending, reason: "needs the template evaluator (M2)")
            ]),
            AppResult(name: "Slack", bundleID: "com.tinyspeck.slackmacgap", cases: [
                CaseResult(.ascii, .pass, attempts: 1, method: "type", latencyMs: 80),
                CaseResult(
                    .undo, .fail, attempts: 3, method: "type", reason: "undo did not bring the abbreviation back"
                )
            ]),
            AppResult(name: "Word", bundleID: "com.microsoft.Word", skipped: "not installed")
        ])
    }

    func testTheSummaryCountsAppsAndCases() {
        XCTAssertEqual(report.passedApps, 1)
        XCTAssertEqual(report.tested.count, 2)
        XCTAssertEqual(report.caseCounts.ran, 4)
        XCTAssertEqual(report.caseCounts.failed, 1)
        XCTAssertEqual(report.latencyMs["type"], [12, 80])
        XCTAssertFalse(report.latencyWithinBudget)
    }

    func testTheTableIsWhatCIPublishes() {
        let markdown = report.markdown(cases: [.ascii, .long, .cursor, .undo])
        XCTAssertTrue(markdown.contains("| TextEdit | pass | pass (attempt 2) | pending | – | pass |"))
        XCTAssertTrue(markdown.contains("| Slack | pass | – | – | **FAIL** | **FAIL** |"))
        XCTAssertTrue(markdown.contains("| Word | – | – | – | – | skipped: not installed |"))
        XCTAssertTrue(markdown.contains("**1 of 3 apps pass.** Tested: 2. 1 of 4 cases failed (25.0%)."))
        XCTAssertTrue(markdown.contains("- Slack, undo: undo did not bring the abbreviation back"))
        XCTAssertTrue(markdown.contains("| type | 2 | 12.0 | 80.0 | 80.0 | 80.0 | **no** |"))
    }

    func testAnAppWithOnlyPendingCasesHasNotPassed() {
        let result = AppResult(name: "App", bundleID: "app", cases: [CaseResult(.cursor, .pending)])
        XCTAssertFalse(result.passed)
    }

    func testALatencyReportHasNoEmptyMatrixSection() {
        let markdown = Report(config: "chosen", apps: [], extraLatencyMs: ["type": [30, 32]]).markdown()
        XCTAssertFalse(markdown.contains("Injection matrix"))
        XCTAssertTrue(markdown.hasPrefix("## Typed to inserted"))
    }

    func testTheReportSurvivesJSON() throws {
        let decoded = try JSONDecoder().decode(Report.self, from: report.json())
        XCTAssertEqual(decoded, report)
    }
}
