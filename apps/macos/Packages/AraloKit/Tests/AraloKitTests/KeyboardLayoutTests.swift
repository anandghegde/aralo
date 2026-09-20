import Carbon.HIToolbox
import XCTest

@testable import AraloKit

final class KeyboardLayoutTests: XCTestCase {
    private let qwerty: [CGKeyCode: String] = [
        CGKeyCode(kVK_ANSI_V): "v", CGKeyCode(kVK_ANSI_Z): "z", CGKeyCode(kVK_ANSI_Period): ".",
        CGKeyCode(kVK_ANSI_Slash): "/"
    ]
    // Dvorak has "v" on the US "." key and "z" on the US "/" key.
    private let dvorak: [CGKeyCode: String] = [
        CGKeyCode(kVK_ANSI_V): "k", CGKeyCode(kVK_ANSI_Z): ";", CGKeyCode(kVK_ANSI_Period): "v",
        CGKeyCode(kVK_ANSI_Slash): "z"
    ]

    func testAUSLayoutKeepsTheUSKeys() {
        let codes = KeyboardLayout.keyCodes(for: ["v", "z"]) { keyCode, _ in self.qwerty[keyCode] }
        XCTAssertEqual(codes, ["v": CGKeyCode(kVK_ANSI_V), "z": CGKeyCode(kVK_ANSI_Z)])
    }

    func testDvorakMovesPasteAndUndo() {
        let codes = KeyboardLayout.keyCodes(for: ["v", "z"]) { keyCode, _ in self.dvorak[keyCode] }
        XCTAssertEqual(codes, ["v": CGKeyCode(kVK_ANSI_Period), "z": CGKeyCode(kVK_ANSI_Slash)])
    }

    func testWhatTheKeyGivesWithCommandHeldWins() {
        // "Dvorak – QWERTY ⌘" types Dvorak but keeps shortcuts on the QWERTY keys.
        let codes = KeyboardLayout.keyCodes(for: ["v", "z"]) { keyCode, command in
            (command ? self.qwerty : self.dvorak)[keyCode]
        }
        XCTAssertEqual(codes, ["v": CGKeyCode(kVK_ANSI_V), "z": CGKeyCode(kVK_ANSI_Z)])
    }

    func testACharacterNoKeyGivesIsLeftOutAndTheSinkFallsBackToTheUSKey() {
        let codes = KeyboardLayout.keyCodes(for: ["v", "z"]) { keyCode, _ in
            keyCode == CGKeyCode(kVK_ANSI_Period) ? "v" : "ж"
        }
        XCTAssertEqual(codes, ["v": CGKeyCode(kVK_ANSI_Period)])

        let shortcuts = ShortcutKeyCodes(codes)
        XCTAssertEqual(SystemEventSink.keyCode(for: .paste, shortcuts: shortcuts), CGKeyCode(kVK_ANSI_Period))
        XCTAssertEqual(SystemEventSink.keyCode(for: .undo, shortcuts: shortcuts), CGKeyCode(kVK_ANSI_Z))
        XCTAssertEqual(SystemEventSink.keyCode(for: .backspace, shortcuts: shortcuts), CGKeyCode(kVK_Delete))
    }

    @MainActor
    func testTheLayoutInUseCanBeRead() throws {
        let codes = KeyboardLayout.currentKeyCodes()
        // A CI runner may have no Unicode layout, and a contributor's layout
        // may have no Latin letters, so only check what came back.
        try XCTSkipIf(codes.isEmpty, "the keyboard layout in use gives none of the shortcut characters")
        XCTAssertTrue(Set(codes.keys).isSubset(of: KeyboardLayout.shortcutCharacters))
        XCTAssertTrue(codes.values.allSatisfy { $0 < 128 })
    }
}
