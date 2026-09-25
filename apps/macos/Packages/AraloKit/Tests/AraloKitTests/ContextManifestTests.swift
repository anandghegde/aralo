import AraloBridge
@testable import AraloKit
import XCTest

/// The manifest a panel shows beside an answer (plan 4.10): every declared
/// kind, what went and what had nothing to send, and where it went.
final class ContextManifestTests: XCTestCase {
    func testNothingIsShownBeforeARequestHasGoneOut() {
        let manifest = ContextManifest([], profile: nil, model: nil)
        XCTAssertFalse(manifest.hasGoneOut)
        XCTAssertEqual(manifest.summary, "")
    }

    func testTheSummaryNamesWhatWentAndTheListWhatDidNot() {
        let manifest = ContextManifest(
            [
                AiContextSent(kind: .selection, bytes: 2048),
                AiContextSent(kind: .clipboard, bytes: nil)
            ],
            profile: "Example", model: "model-a", request: "the instruction"
        )
        XCTAssertEqual(manifest.destination, "Example · model-a")
        XCTAssertEqual(manifest.summary, "Sent the selection (2 KB) to Example · model-a")
        XCTAssertEqual(manifest.lines.map(\.name), ["The selection", "The clipboard"])
        XCTAssertEqual(manifest.lines.map(\.detail), ["2 KB", "nothing to send, so nothing sent"])
        XCTAssertEqual(manifest.closing, "With the instruction. Nothing else was sent.")
    }

    func testARequestWithNoContextSaysWhatItSentAlone() {
        let manifest = ContextManifest([], profile: "Local", model: "llama", request: "the label and your note")
        XCTAssertEqual(manifest.summary, "Sent the label and your note only to Local · llama")
        XCTAssertTrue(manifest.lines.isEmpty)
    }
}
