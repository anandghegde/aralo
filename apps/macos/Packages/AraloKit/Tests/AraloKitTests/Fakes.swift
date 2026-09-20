@testable import AraloKit
import Carbon.HIToolbox
import Foundation

/// A layout that ships with macOS, whether or not the user has enabled it.
@MainActor
func systemLayout(_ inputSourceID: String) -> KeyTranslator.Layout? {
    let filter = [kTISPropertyInputSourceID as String: inputSourceID] as CFDictionary
    let sources = TISCreateInputSourceList(filter, true)?.takeRetainedValue() as? [TISInputSource]
    return sources?.first.flatMap(KeyboardLayout.layout(of:))
}

final class RecordingSink: EventSink {
    private(set) var keys: [SyntheticKey] = []

    func post(_ key: SyntheticKey) {
        keys.append(key)
    }

    /// The text typed so far, with Backspace applied, as a text field would show it.
    var typedText: String {
        var text = ""
        for key in keys {
            if key == .backspace {
                if !text.isEmpty { text.removeLast() }
            } else if let typed = key.text {
                text += typed
            }
        }
        return text
    }
}

final class FakePasteboard: PasteboardAccess {
    private(set) var changeCount = 0
    private(set) var items: PasteboardContents
    private(set) var lastMarkers: [String] = []
    /// What the pasteboard held each time Cmd+V would have read it.
    var onWrite: (() -> Void)?

    init(items: PasteboardContents = []) {
        self.items = items
    }

    func contents() -> PasteboardContents { items }

    func write(text: String, markerTypes: [String]) {
        changeCount += 1
        items = [["public.utf8-plain-text": Data(text.utf8)]]
        lastMarkers = markerTypes
        onWrite?()
    }

    func restore(_ contents: PasteboardContents) {
        changeCount += 1
        items = contents
    }

    /// Another app copies something.
    func copyFromElsewhere(_ text: String) {
        changeCount += 1
        items = [["public.utf8-plain-text": Data(text.utf8)]]
    }
}
