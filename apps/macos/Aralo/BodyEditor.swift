import AppKit
import AraloBridge
import AraloKit
import SwiftUI

/// The snippet's body: a TextKit 2 text view that draws what the core reads in
/// it, an Insert menu of the placeholders the core knows, an AI menu that asks
/// a model to write or change it, and what the core has to say about the
/// placeholders it could not read.
///
/// Every range drawn here comes from the core's own reading of the body, the
/// same one an expansion runs, so what is marked is what would happen (PRD
/// L10). The view holds no opinion about the grammar: it is handed ranges.
struct BodyEditor: View {
    let store: LibraryStore
    @Binding var text: String
    /// The snippet's label, which a draft is written from.
    let label: String

    /// The Insert menu's, the AI menu's and the advice list's way into the
    /// text view, so all of them work on the text the reader's caret is in.
    @State private var editor = BodyEditorHandle()

    /// The AI sheet's model, while it is up.
    @State private var authoring: AuthoringStore?

    /// The languages the Translate menu offers. A model knows many more; these
    /// are the ones asked for most.
    private static let languages = [
        "English", "Spanish", "French", "German", "Italian", "Portuguese", "Dutch", "Japanese",
        "Chinese", "Korean"
    ]

    private var highlight: BodyHighlight { store.outline(of: text) }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text("Body").font(.headline)
                Spacer(minLength: 8)
                if store.writer != nil {
                    aiMenu
                }
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
        .sheet(isPresented: authoringIsUp) {
            if let authoring {
                AuthoringSheet(
                    store: authoring,
                    replace: { answer in
                        editor.replace(
                            authoring.range, expected: authoring.replacing, with: answer, action: authoring.label
                        )
                    },
                    close: closeAuthoring
                )
            }
        }
    }

    /// Asks a model to write or change the body. What it works on, and all
    /// that is sent, is the selection, or the whole body when nothing is
    /// selected; a draft sends only the label and the note.
    private var aiMenu: some View {
        Menu {
            Button("Draft with AI\u{2026}") { ask(.draft(label: label, note: "")) }
            Divider()
            Button("Fix Spelling and Grammar") { ask(.proofread) }
            Button("Make It Clearer") { ask(.clearer) }
            Button("Make It Shorter") { ask(.shorter) }
            Menu("Change the Tone") {
                Button("Friendlier") { ask(.friendlier) }
                Button("More Formal") { ask(.formal) }
                Button("More Casual") { ask(.casual) }
            }
            Menu("Translate") {
                ForEach(Self.languages, id: \.self) { language in
                    Button(language) { ask(.translate(language: language)) }
                }
            }
            Button("Suggest Variations") { ask(.variations) }
        } label: {
            Label("AI", systemImage: "sparkles")
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .help("Ask a model to write or change the body. The selection, or the whole body, is what it sees.")
    }

    private func ask(_ action: AiAuthoring) {
        guard let writer = store.writer, let target = editor.target() else { return }
        authoring = AuthoringStore(action: action, replacing: target.text, range: target.range, runner: writer)
    }

    private func closeAuthoring() {
        authoring?.close()
        authoring = nil
    }

    /// The sheet is up for as long as its model is. Closing it any other way
    /// is closing it.
    private var authoringIsUp: Binding<Bool> {
        Binding(
            get: { authoring != nil },
            set: { isUp in
                if !isUp { closeAuthoring() }
            }
        )
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
                            .accessibilityLabel(item.isError ? "Error" : "Note")
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

    /// What an AI action works on: the selection, or the whole body when
    /// nothing is selected. Nil before the view is there.
    func target() -> (range: NSRange, text: String)? {
        guard let view else { return nil }
        let whole = view.string as NSString
        let selected = view.selectedRange()
        let range = selected.length > 0 ? selected : NSRange(location: 0, length: whole.length)
        return (range, whole.substring(with: range))
    }

    /// Puts an AI answer in place of `range`, as one edit the editor's undo
    /// takes back. False when the range no longer holds `expected`.
    func replace(_ range: NSRange, expected: String, with text: String, action: String) -> Bool {
        guard let view else { return false }
        return TextReplacement.replace(in: view, range: range, expected: expected, with: text, action: action)
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
        view.setAccessibilityLabel("Body")
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
