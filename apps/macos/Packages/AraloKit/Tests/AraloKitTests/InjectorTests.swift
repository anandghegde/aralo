import AraloBridge
@testable import AraloKit
import XCTest

final class InjectorTests: XCTestCase {
    private func makeInjector(_ sink: RecordingSink, _ pasteboard: FakePasteboard) -> Injector {
        Injector(sink: sink, pasteboard: pasteboard, sleep: { _ in })
    }

    /// The table's defaults, with whatever the test is about changed.
    private func profile(_ change: (inout InjectionProfile) -> Void = { _ in }) -> InjectionProfile {
        var profile = InjectionProfile(
            insert: .auto, typingLimit: 120, keyDelayMs: 1, pasteSettleMs: 250, undo: .native, delete: .backspace
        )
        change(&profile)
        return profile
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

    func testAutoTypesShortSingleLineTextAndPastesEverythingElse() {
        XCTAssertEqual(Injector.method(for: "thank you ", profile: profile()), .typed)
        XCTAssertEqual(Injector.method(for: String(repeating: "a", count: 120), profile: profile()), .typed)
        XCTAssertEqual(Injector.method(for: String(repeating: "a", count: 121), profile: profile()), .pasted)
        XCTAssertEqual(Injector.method(for: "Best regards,\nSam", profile: profile()), .pasted)

        let shortLimit = profile { $0.typingLimit = 4 }
        XCTAssertEqual(Injector.method(for: "four", profile: shortLimit), .typed)
        XCTAssertEqual(Injector.method(for: "fiver", profile: shortLimit), .pasted)
    }

    func testTheProfileCanForceEitherMethod() {
        let long = String(repeating: "a", count: 500) + "\nb"
        XCTAssertEqual(Injector.method(for: long, profile: profile { $0.insert = .type }), .typed)
        XCTAssertEqual(Injector.method(for: "ok", profile: profile { $0.insert = .paste }), .pasted)
    }

    func testForcedTypingSendsLineBreaksAsReturnAndGivesUpUndo() {
        let sink = RecordingSink()
        let pasteboard = FakePasteboard()
        let typing = profile { $0.insert = .type }
        let outcome = makeInjector(sink, pasteboard).run([.insertText(text: "cd /tmp\r\nls\n")], profile: typing)
        XCTAssertEqual(sink.keys, [.text("cd /tmp"), .return, .text("ls"), .return])
        XCTAssertEqual(outcome, Injector.Outcome(method: .typed, undoable: false))
        XCTAssertEqual(pasteboard.changeCount, 0, "the clipboard is never touched")

        let single = makeInjector(RecordingSink(), pasteboard).run([.insertText(text: "ls -la")], profile: typing)
        XCTAssertEqual(single, Injector.Outcome(method: .typed, undoable: true))
    }

    func testForcedPasteAppliesToShortTextToo() {
        let sink = RecordingSink()
        let outcome = makeInjector(sink, FakePasteboard()).run(
            [.delete(count: 2), .insertText(text: "ok")], profile: profile { $0.insert = .paste }
        )
        XCTAssertEqual(sink.keys, [.backspace, .backspace, .paste])
        XCTAssertEqual(outcome, Injector.Outcome(method: .pasted, undoable: true))
    }

    func testSelectDeleteExtendsTheSelectionThenDeletesOnce() {
        let sink = RecordingSink()
        let selecting = profile { $0.delete = .select }
        let injector = makeInjector(sink, FakePasteboard())
        injector.run([.delete(count: 3), .insertText(text: "hi"), .delete(count: 0)], profile: selecting)
        let select = SyntheticKey.leftArrow(selecting: true)
        XCTAssertEqual(sink.keys, [select, select, select, .backspace, .text("hi")])

        let undone = RecordingSink()
        makeInjector(undone, FakePasteboard()).undo(deleteCount: 2, retype: "h ", method: .typed, profile: selecting)
        XCTAssertEqual(undone.keys, [select, select, .backspace, .text("h ")])
    }

    func testTheProfileSetsThePauses() {
        var pauses: [TimeInterval] = []
        let injector = Injector(sink: RecordingSink(), pasteboard: FakePasteboard()) { pauses.append($0) }
        let slow = profile {
            $0.keyDelayMs = 8
            $0.pasteSettleMs = 400
        }
        injector.run([.delete(count: 1), .insertText(text: "a\nb")], profile: slow)
        XCTAssertEqual(pauses, [0.008, 0.008, 0.4])
    }

    func testTypedPlanDeletesThenTypesThenPressesKeys() {
        let sink = RecordingSink()
        let injector = makeInjector(sink, FakePasteboard())
        let outcome = injector.run([
            .delete(count: 2),
            .insertText(text: "on my way"),
            .keyPress(key: .return),
            .moveCursor(graphemes: 2, select: true)
        ], profile: profile())
        XCTAssertEqual(outcome, Injector.Outcome(method: .typed, undoable: true))
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

        let outcome = makeInjector(sink, pasteboard).run([.insertText(text: "line one\nline two")], profile: profile())

        XCTAssertEqual(outcome, Injector.Outcome(method: .pasted, undoable: true))
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
        injector.run([.insertText(text: "a\nb")], profile: profile())
        XCTAssertEqual(pasteboard.contents(), [["public.utf8-plain-text": Data("newer".utf8)]])
    }

    func testUndoOfTypedTextBackspacesAndOfPastedTextForwardsOneUndo() {
        let typed = RecordingSink()
        makeInjector(typed, FakePasteboard()).undo(deleteCount: 3, retype: "ty ", method: .typed, profile: profile())
        XCTAssertEqual(typed.keys, [.backspace, .backspace, .backspace, .text("ty ")])

        let pasted = RecordingSink()
        makeInjector(pasted, FakePasteboard())
            .undo(deleteCount: 300, retype: ";addr ", method: .pasted, profile: profile())
        XCTAssertEqual(pasted.keys, [.undo, .text(";addr ")])
    }

    func testWhereTheAppsOwnUndoCannotBeTrustedAPasteIsBackspacedToo() {
        let sink = RecordingSink()
        makeInjector(sink, FakePasteboard())
            .undo(deleteCount: 4, retype: "a ", method: .pasted, profile: profile { $0.undo = .backspace })
        XCTAssertEqual(sink.keys, [.backspace, .backspace, .backspace, .backspace, .text("a ")])
    }
}
