@testable import AraloKit
import XCTest

final class SecureInputTests: XCTestCase {
    func testTheHolderIsReadFromTheSessionDictionary() {
        XCTAssertEqual(SecureInputMonitor.holderPID(in: ["kCGSSessionSecureInputPID": NSNumber(value: 4242)]), 4242)
        XCTAssertEqual(SecureInputMonitor.holderPID(in: ["kCGSSessionSecureInputPID": 77]), 77)
    }

    func testAMissingOrOddEntryNamesNobody() {
        XCTAssertNil(SecureInputMonitor.holderPID(in: [:]))
        XCTAssertNil(SecureInputMonitor.holderPID(in: ["kCGSSessionSecureInputPID": 0]))
        XCTAssertNil(SecureInputMonitor.holderPID(in: ["kCGSSessionSecureInputPID": "4242"]))
        XCTAssertNil(SecureInputMonitor.holderPID(in: ["kCGSSessionOnConsoleKey": true]))
    }

    @MainActor
    func testNobodyIsNamedWhileSecureInputIsOff() throws {
        let monitor = SecureInputMonitor { _ in }
        try XCTSkipIf(monitor.isSecureInputOn, "something on this machine holds secure input")
        XCTAssertNil(monitor.holder)
    }
}
