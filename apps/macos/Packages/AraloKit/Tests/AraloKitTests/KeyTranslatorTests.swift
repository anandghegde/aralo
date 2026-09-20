import AraloBridge
import Carbon.HIToolbox
import XCTest

@testable import AraloKit

@MainActor
final class KeyTranslatorTests: XCTestCase {
    private func translator(_ inputSourceID: String) throws -> KeyTranslator {
        KeyTranslator(layout: try XCTUnwrap(systemLayout(inputSourceID), "\(inputSourceID) is not installed"))
    }

    private func key(_ code: Int) -> CGKeyCode { CGKeyCode(code) }

    func testPlainKeysFollowTheLayoutAndTheModifiers() throws {
        let usLayout = try translator("com.apple.keylayout.US")
        XCTAssertEqual(usLayout.text(keyCode: key(kVK_ANSI_A), flags: []), "a")
        XCTAssertEqual(usLayout.text(keyCode: key(kVK_ANSI_A), flags: .maskShift), "A")
        XCTAssertEqual(usLayout.text(keyCode: key(kVK_ANSI_A), flags: .maskAlphaShift), "A")
        XCTAssertEqual(usLayout.text(keyCode: key(kVK_ANSI_2), flags: .maskShift), "@")
        XCTAssertEqual(usLayout.text(keyCode: key(kVK_Space), flags: []), " ")

        let french = try translator("com.apple.keylayout.French")
        XCTAssertEqual(french.text(keyCode: key(kVK_ANSI_A), flags: []), "q")
    }

    func testADeadKeyGivesNothingAndThenTheAccentedLetter() throws {
        let usLayout = try translator("com.apple.keylayout.US")
        XCTAssertEqual(usLayout.text(keyCode: key(kVK_ANSI_E), flags: .maskAlternate), "")
        XCTAssertTrue(usLayout.isDeadKeyPending)
        XCTAssertEqual(usLayout.text(keyCode: key(kVK_ANSI_E), flags: []), "é")
        XCTAssertFalse(usLayout.isDeadKeyPending)
        // The pair is over: the next "e" is just an "e".
        XCTAssertEqual(usLayout.text(keyCode: key(kVK_ANSI_E), flags: []), "e")

        let german = try translator("com.apple.keylayout.German")
        XCTAssertEqual(german.text(keyCode: key(kVK_ANSI_Equal), flags: []), "")
        XCTAssertEqual(german.text(keyCode: key(kVK_ANSI_A), flags: .maskShift), "Á")
    }

    func testAnAccentThatDoesNotCombineComesOutWithTheLetter() throws {
        let usLayout = try translator("com.apple.keylayout.US")
        XCTAssertEqual(usLayout.text(keyCode: key(kVK_ANSI_E), flags: .maskAlternate), "")
        XCTAssertEqual(usLayout.text(keyCode: key(kVK_ANSI_X), flags: []), "´x")
        let stroke = KeyStroke(keyCode: key(kVK_ANSI_X), text: "´x")
        XCTAssertEqual(KeyClassifier.classify(stroke), .reset(.unmappableInput))
    }

    func testClearAndANewLayoutForgetAPendingDeadKey() throws {
        let usLayout = try translator("com.apple.keylayout.US")
        XCTAssertFalse(usLayout.clear())
        XCTAssertEqual(usLayout.text(keyCode: key(kVK_ANSI_E), flags: .maskAlternate), "")
        XCTAssertTrue(usLayout.clear())
        XCTAssertEqual(usLayout.text(keyCode: key(kVK_ANSI_E), flags: []), "e")

        XCTAssertEqual(usLayout.text(keyCode: key(kVK_ANSI_E), flags: .maskAlternate), "")
        usLayout.replace(layout: nil, composing: true)
        XCTAssertFalse(usLayout.isDeadKeyPending)
        XCTAssertTrue(usLayout.isComposing)
        // No layout: the caller uses the event's own string.
        XCTAssertNil(usLayout.text(keyCode: key(kVK_ANSI_E), flags: []))
    }

    func testOnlyAComposingInputMethodPausesMatching() {
        let layout = kTISTypeKeyboardLayout as String
        let mode = kTISTypeKeyboardInputMode as String
        XCTAssertFalse(KeyboardLayout.composes(type: layout, inputMode: nil))
        XCTAssertFalse(KeyboardLayout.composes(type: nil, inputMode: nil))
        XCTAssertTrue(KeyboardLayout.composes(type: mode, inputMode: "com.apple.inputmethod.Japanese"))
        XCTAssertTrue(KeyboardLayout.composes(type: kTISTypeKeyboardInputMethodWithoutModes as String, inputMode: nil))
        XCTAssertFalse(KeyboardLayout.composes(type: mode, inputMode: "com.apple.inputmethod.Roman"))
    }
}
