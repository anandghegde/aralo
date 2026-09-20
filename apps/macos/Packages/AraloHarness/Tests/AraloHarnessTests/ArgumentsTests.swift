@testable import AraloHarness
import XCTest

final class ArgumentsTests: XCTestCase {
    func testDefaultsAreTheNightlyRun() throws {
        let arguments = try Arguments(["matrix"])
        XCTAssertEqual(arguments.config, .chosen)
        XCTAssertEqual(arguments.cases, MatrixCase.allCases)
        XCTAssertEqual(arguments.attempts, 3)
        XCTAssertEqual(arguments.manualWait, 0)
        XCTAssertTrue(arguments.launchAralo)
    }

    func testOptionsAreRead() throws {
        let arguments = try Arguments([
            "matrix", "--method", "paste", "--undo", "backspace", "--only", "com.apple.TextEdit,com.apple.Notes",
            "--cases", "ascii,undo", "--require", "13", "--manual-wait", "20", "--no-launch", "--json", "out.json"
        ])
        XCTAssertEqual(arguments.config, .paste)
        XCTAssertEqual(arguments.undo, "backspace")
        XCTAssertEqual(arguments.only, ["com.apple.TextEdit", "com.apple.Notes"])
        XCTAssertEqual(arguments.cases, [.ascii, .undo])
        XCTAssertEqual(arguments.require, 13)
        XCTAssertEqual(arguments.manualWait, 20)
        XCTAssertFalse(arguments.launchAralo)
        XCTAssertEqual(arguments.json, "out.json")
    }

    func testMistakesAreNamed() {
        for words in [[], ["dance"], ["matrix", "--method", "guess"], ["matrix", "--cases", "ascii,nope"],
                      ["latency", "--runs"], ["latency", "--runs", "many"], ["matrix", "--loud"]] {
            XCTAssertThrowsError(try Arguments(words), "\(words)") { XCTAssertTrue($0 is Arguments.Problem) }
        }
    }

    func testKeyChordsAreReadAndWrittenTheSameWay() {
        XCTAssertEqual(KeyChord("cmd+n"), KeyChord(.character("n"), [.cmd]))
        XCTAssertEqual(KeyChord("Ctrl+U"), .killLine)
        XCTAssertEqual(KeyChord("shift+cmd+z")?.description, "cmd+shift+z")
        XCTAssertEqual(KeyChord("tab"), KeyChord(.tab))
        XCTAssertEqual(KeyChord("enter"), KeyChord(.return))
        for bad in ["", "cmd+", "cmd+cmd+a", "hyper+a", "cmd+tabby"] {
            XCTAssertNil(KeyChord(bad), bad)
        }
    }
}
