import AppKit
import AraloKit
import SwiftUI

/// The snippet's body: a TextKit 2 text view that draws what the core reads in
/// it, an Insert menu of the placeholders the core knows, and what the core has
/// to say about the ones it could not read.
///
/// Every range drawn here comes from the core's own reading of the body, the
/// same one an expansion runs, so what is marked is what would happen (PRD
/// L10). The view holds no opinion about the grammar: it is handed ranges.
struct BodyEditor: View {
    let store: LibraryStore
    @Binding var text: String

    /// The Insert menu's and the advice list's way into the text view, so both
    /// work on the text the reader's caret is in.
    @State private var editor = BodyEditorHandle()

    private var highlight: BodyHighlight { store.outline(of: text) }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text("Body").font(.headline)
                Spacer(minLength: 8)
                insertMenu
            }
            HighlightingTextView(text: $text, highlight: highlight, handle: editor)
                .frame(minHeight: 180)
                .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 6))
            if highlight.advice.isEmpty {
                Text("A placeholder is written \u{7B}\u{7B}like this\u{7D}\u{7D}. Insert lists the ones Aralo knows.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                advice
            }
        }
    }

    private var insertMenu: some View {
        Menu {
            ForEach(Placeholder.all) { choice in
                Button("\u{7B}\u{7B}\(choice.name)\u{7D}\u{7D}") { editor.insert(choice) }
                    .help(choice.summary)
            }
        } label: {
            Label("Insert", systemImage: "plus.circle")
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .help("Put a placeholder in where the caret is")
    }

    /// What the core could not read, in its words. Clicking one shows the part
    /// of the body it is about.
    private var advice: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(highlight.advice) { item in
                Button {
                    editor.select(item.range)
                } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Image(systemName: item.isError ? "exclamationmark.triangle.fill" : "info.circle")
                            .foregroundStyle(item.isError ? .red : .orange)
                        Text(item.message)
                            .multilineTextAlignment(.leading)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                .buttonStyle(.plain)
                .help("Show this in the body")
            }
        }
        .font(.callout)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A hold on the text view the reader is typing in, for the parts of the editor
/// that are outside it. AppKit does the insertion, so undo takes a placeholder
/// back the way it takes back typing.
@MainActor
final class BodyEditorHandle {
    fileprivate weak var view: NSTextView?

    nonisolated init() {}

    /// Puts a placeholder in where the caret is, over the selection when there
    /// is one, and selects the part the reader replaces with their own words.
    func insert(_ placeholder: Placeholder) {
        guard let view else { return }
        view.window?.makeFirstResponder(view)
        let replacing = view.selectedRange()
        view.insertText(placeholder.insert, replacementRange: replacing)
        view.setSelectedRange(placeholder.selection(insertedAt: replacing.location))
        view.scrollRangeToVisible(view.selectedRange())
    }

    /// Shows the reader the range a problem is about.
    func select(_ range: NSRange) {
        guard let view, NSMaxRange(range) <= (view.string as NSString).length else { return }
        view.window?.makeFirstResponder(view)
        view.setSelectedRange(range)
        view.scrollRangeToVisible(range)
    }
}

/// The text view itself. TextKit 2, plain text, and its own undo.
private struct HighlightingTextView: NSViewRepresentable {
    @Binding var text: String
    let highlight: BodyHighlight
    let handle: BodyEditorHandle

    func makeNSView(context: Context) -> NSScrollView {
        let view = NSTextView(usingTextLayoutManager: true)
        view.delegate = context.coordinator
        view.isRichText = false
        view.allowsUndo = true
        view.drawsBackground = false
        view.textContainerInset = NSSize(width: 6, height: 8)
        view.font = Self.bodyFont
        view.typingAttributes = Self.base
        // A snippet body is typed out verbatim: a substituted quote or dash
        // would change what the reader's app receives. Spelling underlines are
        // off too, so the only marks on the body are the core's.
        view.isAutomaticQuoteSubstitutionEnabled = false
        view.isAutomaticDashSubstitutionEnabled = false
        view.isAutomaticTextReplacementEnabled = false
        view.isAutomaticSpellingCorrectionEnabled = false
        view.isContinuousSpellCheckingEnabled = false
        view.isGrammarCheckingEnabled = false
        view.minSize = NSSize(width: 0, height: 0)
        view.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        view.isVerticallyResizable = true
        view.isHorizontallyResizable = false
        view.autoresizingMask = .width
        view.textContainer?.widthTracksTextView = true
        view.textContainer?.size = NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude)
        view.string = text

        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.borderType = .noBorder
        scroll.drawsBackground = false
        scroll.documentView = view

        handle.view = view
        Self.draw(highlight, in: view)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let view = scroll.documentView as? NSTextView else { return }
        context.coordinator.text = $text
        handle.view = view
        // A change from anywhere but the keyboard: a revert, or the file being
        // read again. The caret keeps its place in the new text.
        if !context.coordinator.isSending, view.string != text {
            let caret = view.selectedRange().location
            view.string = text
            view.setSelectedRange(NSRange(location: min(caret, (text as NSString).length), length: 0))
        }
        Self.draw(highlight, in: view)
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        var text: Binding<String>
        /// True while the typed text is on its way to the binding, so the
        /// change coming back is not written over the caret.
        private(set) var isSending = false

        init(text: Binding<String>) {
            self.text = text
        }

        func textDidChange(_ notification: Notification) {
            guard let view = notification.object as? NSTextView else { return }
            isSending = true
            text.wrappedValue = view.string
            isSending = false
        }
    }

    /// Paints the core's ranges over the body. The attributes are set afresh
    /// each time rather than tracked: a snippet body is short, and a run that
    /// has moved must not leave its colour behind.
    private static func draw(_ highlight: BodyHighlight, in view: NSTextView) {
        // While an input method is composing, the marked text carries its own
        // underline and is not settled text yet. Painting over it would break
        // the composition, and the next keystroke redraws anyway.
        guard let storage = view.textStorage, !view.hasMarkedText() else { return }
        let whole = NSRange(location: 0, length: storage.length)
        storage.beginEditing()
        storage.setAttributes(base, range: whole)
        for run in highlight.runs {
            let range = NSIntersectionRange(run.range, whole)
            guard range.length > 0 else { continue }
            storage.addAttributes(attributes(for: run.style), range: range)
        }
        storage.endEditing()
        view.typingAttributes = base
    }

    private static var bodyFont: NSFont {
        .monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
    }

    private static var base: [NSAttributedString.Key: Any] {
        [.font: bodyFont, .foregroundColor: NSColor.labelColor]
    }

    private static func attributes(for style: BodyStyle) -> [NSAttributedString.Key: Any] {
        switch style {
        case .placeholder:
            return [.foregroundColor: NSColor.controlAccentColor]
        // It reads as a placeholder but nothing answers to the name, so it is
        // drawn as the literal text it will expand to.
        case .unknownPlaceholder:
            return [.foregroundColor: NSColor.secondaryLabelColor]
        case .error:
            return [
                .underlineStyle: NSUnderlineStyle.single.rawValue,
                .underlineColor: NSColor.systemRed
            ]
        case .note:
            return [
                .underlineStyle: NSUnderlineStyle.single.rawValue | NSUnderlineStyle.patternDot.rawValue,
                .underlineColor: NSColor.systemOrange
            ]
        }
    }
}
