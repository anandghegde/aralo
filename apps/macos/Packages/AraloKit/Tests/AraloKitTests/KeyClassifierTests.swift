import AraloBridge
@testable import AraloKit
import Carbon.HIToolbox
import XCTest

final class KeyClassifierTests: XCTestCase {
    private func classify(_ text: String, keyCode: Int = kVK_ANSI_A, flags: CGEventFlags = []) -> KeyClassification {
        KeyClassifier.classify(KeyStroke(keyCode: CGKeyCode(keyCode), flags: flags, text: text))
    }

    func testCharactersComeFromTheLayoutNotTheKeyCode() {
        XCTAssertEqual(classify("a"), .input(.char(scalar: 0x61)))
        XCTAssertEqual(classify("é"), .input(.char(scalar: 0xE9)))
        XCTAssertEqual(classify("™", flags: .maskAlternate), .input(.char(scalar: 0x2122)))
        XCTAssertEqual(classify("😀"), .input(.char(scalar: 0x1F600)))
    }

    func testReturnEnterAndTabAreDelimiters() {
        XCTAssertEqual(classify("\r", keyCode: kVK_Return), .input(.char(scalar: 0x0D)))
        XCTAssertEqual(classify("\u{3}", keyCode: kVK_ANSI_KeypadEnter), .input(.char(scalar: 0x0D)))
        XCTAssertEqual(classify("\t", keyCode: kVK_Tab), .input(.char(scalar: 0x09)))
    }

    func testADeadKeyOnItsOwnIsIgnoredAndSeveralScalarsReset() {
        XCTAssertEqual(classify(""), .ignore)
        XCTAssertEqual(classify("e\u{301}"), .reset(.unmappableInput))
    }

    func testShortcutsResetExceptPlainUndo() {
        XCTAssertEqual(classify("z", keyCode: kVK_ANSI_Z, flags: .maskCommand), .input(.undo))
        // Dvorak: the Z character sits on another key.
        XCTAssertEqual(classify("z", keyCode: kVK_ANSI_Slash, flags: .maskCommand), .input(.undo))
        XCTAssertEqual(classify("z", flags: [.maskCommand, .maskShift]), .reset(.shortcut))
        XCTAssertEqual(classify("c", flags: .maskCommand), .reset(.shortcut))
        XCTAssertEqual(classify("a", flags: .maskControl), .reset(.shortcut))
    }

    func testNavigationResets() {
        for keyCode in [kVK_LeftArrow, kVK_UpArrow, kVK_Home, kVK_PageDown, kVK_ForwardDelete, kVK_Escape] {
            XCTAssertEqual(classify("\u{F702}", keyCode: keyCode), .reset(.navigation), "\(keyCode)")
        }
        XCTAssertEqual(classify("\u{F704}", keyCode: kVK_F1), .reset(.navigation))
    }

    func testOnlyPlainDeleteIsBackspace() {
        XCTAssertEqual(classify("\u{7F}", keyCode: kVK_Delete), .input(.backspace))
        XCTAssertEqual(classify("\u{7F}", keyCode: kVK_Delete, flags: .maskAlternate), .reset(.navigation))
    }
}
