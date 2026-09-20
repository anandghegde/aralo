import AraloBridge
@testable import AraloKit
import XCTest

final class InjectorTests: XCTestCase {
    private func makeInjector(_ sink: RecordingSink, _ pasteboard: FakePasteboard) -> Injector {
        Injector(sink: sink, pasteboard: pasteboard, sleep: { _ in })
    }

    func testChunksStayUnderTwentyUnitsAndNeverSplitACharacter() {
        let family = "👨‍👩‍👧‍👦" // 11 UTF-16 units, one character
        let text = String(repeating: "ab", count: 9) + family + "e\u{301}" + String(repeating: "x", count: 25)
        let chunks = Injector.chunks(of: text)
        XCTAssertEqual(chunks.joined(), text)
        XCTAssertTrue(chunks.allSatisfy { $0.utf16.count <= 20 }, "\(chunks.map(\.utf16.count))")
        XCTAssertTrue(chunks.contains { $0.contains(family) })

        // A single character longer than the limit travels alone rather than in pieces.
        let long = "e" + String(repeating: "\u{301}", count: 30)
        XCTAssertEqual(Injector.chunks(of: "a" + long + "b"), ["a", long, "b"])
        XCTAssertEqual(Injector.chunks(of: ""), [])
    }

    func testShortSingleLineTextIsTypedEverythingElseIsPasted() {
        XCTAssertEqual(Injector.method(for: "thank you "), .typed)
        XCTAssertEqual(Injector.method(for: String(repeating: "a", count: 120)), .typed)
        XCTAssertEqual(Injector.method(for: String(repeating: "a", count: 121)), .pasted)
        XCTAssertEqual(Injector.method(for: "Best regards,\nSam"), .pasted)
    }

    func testTypedPlanDeletesThenTypesThenPressesKeys() {
        let sink = RecordingSink()
        let injector = makeInjector(sink, FakePasteboard())
        let method = injector.run([
            .delete(count: 2),
            .insertText(text: "on my way"),
            .keyPress(key: .return),
            .moveCursor(graphemes: 2, select: true)
        ])
        XCTAssertEqual(method, .typed)
        XCTAssertEqual(sink.keys, [
            .backspace, .backspace, .text("on my way"), .return,
            .leftArrow(selecting: true), .leftArrow(selecting: true)
        ])
    }

    func testPasteMarksTheEntryTransientAndRestoresEveryItemAndType() {
        let original: PasteboardContents = [
            ["public.utf8-plain-text": Data("mine".utf8), "public.rtf": Data([1, 2, 3])],
            ["public.png": Data([9])]
        ]
        let pasteboard = FakePasteboard(items: original)
        let sink = RecordingSink()
        var seenByTheApp: PasteboardContents = []
        pasteboard.onWrite = { seenByTheApp = pasteboard.contents() }

        let method = makeInjector(sink, pasteboard).run([.insertText(text: "line one\nline two")])

        XCTAssertEqual(method, .pasted)
        XCTAssertEqual(sink.keys, [.paste])
        XCTAssertEqual(seenByTheApp, [["public.utf8-plain-text": Data("line one\nline two".utf8)]])
        XCTAssertEqual(pasteboard.lastMarkers, PasteboardMarker.all)
        XCTAssertEqual(pasteboard.contents(), original)
    }

    func testPasteDoesNotRestoreOverSomethingCopiedInTheMeantime() {
        let pasteboard = FakePasteboard(items: [["public.utf8-plain-text": Data("old".utf8)]])
        let injector = Injector(sink: RecordingSink(), pasteboard: pasteboard) { _ in
            // While the injector waits for the app to paste, the user copies.
            if pasteboard.lastMarkers.isEmpty == false, pasteboard.changeCount == 1 {
                pasteboard.copyFromElsewhere("newer")
            }
        }
        injector.run([.insertText(text: "a\nb")])
        XCTAssertEqual(pasteboard.contents(), [["public.utf8-plain-text": Data("newer".utf8)]])
    }

    func testUndoOfTypedTextBackspacesAndOfPastedTextForwardsOneUndo() {
        let typed = RecordingSink()
        makeInjector(typed, FakePasteboard()).undo(deleteCount: 3, retype: "ty ", method: .typed)
        XCTAssertEqual(typed.keys, [.backspace, .backspace, .backspace, .text("ty ")])

        let pasted = RecordingSink()
        makeInjector(pasted, FakePasteboard()).undo(deleteCount: 300, retype: ";addr ", method: .pasted)
        XCTAssertEqual(pasted.keys, [.undo, .text(";addr ")])
    }
}
