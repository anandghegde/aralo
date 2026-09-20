@testable import AraloKit
import Carbon.HIToolbox
import XCTest

@MainActor
final class GlobalHotKeyTests: XCTestCase {
    func testAShortcutIsWrittenTheWayMenusWriteIt() {
        XCTAssertEqual(GlobalHotKey.Shortcut.pause.display, "⌃⌥⌘P")
        let shifted = GlobalHotKey.Shortcut(character: "k", modifiers: [.command, .shift])
        XCTAssertEqual(shifted.display, "⇧⌘K")
    }

    func testModifiersBecomeTheirCarbonBits() {
        XCTAssertEqual(GlobalHotKey.carbonModifiers([]), 0)
        XCTAssertEqual(GlobalHotKey.carbonModifiers(.command), UInt32(cmdKey))
        XCTAssertEqual(
            GlobalHotKey.carbonModifiers(GlobalHotKey.Shortcut.pause.modifiers),
            UInt32(controlKey | optionKey | cmdKey)
        )
        // Caps Lock and Fn are not part of a shortcut.
        XCTAssertEqual(GlobalHotKey.carbonModifiers([.shift, .capsLock, .function]), UInt32(shiftKey))
    }

    func testRegisteringTwiceReplacesAndUnregisteringIsSafeToRepeat() throws {
        let hotKey = GlobalHotKey {}
        XCTAssertFalse(hotKey.isRegistered)
        // F19 with four modifiers: nothing else will own it.
        let modifiers: NSEvent.ModifierFlags = [.control, .option, .command, .shift]
        try XCTSkipUnless(
            hotKey.register(keyCode: CGKeyCode(kVK_F19), modifiers: modifiers), "no window server to register with"
        )
        XCTAssertTrue(hotKey.register(keyCode: CGKeyCode(kVK_F18), modifiers: modifiers))
        XCTAssertTrue(hotKey.isRegistered)
        hotKey.unregister()
        hotKey.unregister()
        XCTAssertFalse(hotKey.isRegistered)
    }
}
