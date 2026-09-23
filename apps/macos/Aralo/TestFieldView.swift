import AppKit
import AraloBridge
import AraloKit
import SwiftUI

/// The editor's test field: type the abbreviation and watch the draft expand,
/// before anything is saved.
///
/// The text view is a window onto a field the core holds. Every key goes to
/// the core, which runs the draft through the same matcher and expansion path
/// as typing into any app, and the view draws what comes back. A form the
/// draft asks for opens as a sheet over the window.
struct TestFieldView: View {
    let store: LibraryStore
    /// What the field is built from. A change to any of it starts the field
    /// again: what was typed was typed against a different draft.
    let editing: Editing

    @State private var field: TestField?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Try It").font(.headline)
                Spacer()
                Button("Clear") { field?.clear() }
                    .buttonStyle(.borderless)
                    .disabled(field?.field.text.isEmpty ?? true)
            }
            if let field {
                TrialTextView(field: field)
                    .frame(height: 72)
                    .background(.background, in: RoundedRectangle(cornerRadius: 6))
                    .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator))
                    .sheet(isPresented: formIsUp(field)) {
                        if let form = field.form {
                            FormView(
                                session: form,
                                submit: { field.submitForm() },
                                cancel: { field.cancelForm() }
                            )
                                .modifier(FormKeyboard(field: field))
                        }
                    }
            }
            Text(caption)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .onAppear { rebuild() }
        .onChange(of: editing.draft) { rebuild() }
        .onChange(of: editing.group) { rebuild() }
        .onChange(of: editing.id) { rebuild() }
        .onDisappear {
            field?.close()
            field = nil
        }
    }

    private var caption: String {
        if field?.abbreviations == 0 {
            return "Give the snippet an abbreviation to try it here."
        }
        return "Type the abbreviation to see what it does. Nothing is saved, and no other app sees it."
    }

    /// The sheet is up for as long as the form is. Closing it any other way is
    /// cancelling the form.
    private func formIsUp(_ field: TestField) -> Binding<Bool> {
        Binding(
            get: { field.form != nil },
            set: { isUp in
                if !isUp { field.cancelForm() }
            }
        )
    }

    private func rebuild() {
        field?.close()
        field = store.testField(clipboard: { NSPasteboard.general.string(forType: .string) })
    }
}

/// The form's boxes hold the keyboard while its sheet is the key window, and
/// the live tap stays out of them as it does out of the field.
private struct FormKeyboard: ViewModifier {
    let field: TestField
    @Environment(\.controlActiveState) private var state

    func body(content: Content) -> some View {
        content
            .onAppear { field.setFormHasKeyboard(state == .key) }
            .onChange(of: state) { field.setFormHasKeyboard(state == .key) }
            .onDisappear { field.setFormHasKeyboard(false) }
    }
}

/// A text view whose text is the core's. It sends keys and caret moves, and
/// never edits itself.
private struct TrialTextView: NSViewRepresentable {
    let field: TestField

    func makeNSView(context: Context) -> NSScrollView {
        let view = SandboxTextView(frame: .zero)
        view.field = field
        view.isRichText = false
        view.allowsUndo = false
        view.drawsBackground = false
        view.font = .monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        view.textContainerInset = NSSize(width: 6, height: 6)
        view.isAutomaticQuoteSubstitutionEnabled = false
        view.isAutomaticDashSubstitutionEnabled = false
        view.isAutomaticTextReplacementEnabled = false
        view.isAutomaticSpellingCorrectionEnabled = false
        view.isContinuousSpellCheckingEnabled = false
        view.isGrammarCheckingEnabled = false
        view.minSize = NSSize(width: 0, height: 0)
        view.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        view.isVerticallyResizable = true
        view.autoresizingMask = .width
        view.textContainer?.widthTracksTextView = true
        view.setAccessibilityLabel("Test field")

        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.documentView = view
        view.redraw()
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let view = scroll.documentView as? SandboxTextView else { return }
        if view.field !== field {
            view.field?.setViewHasKeyboard(false)
            view.field = field
            view.reportKeyboard()
        }
        view.redraw(keepingSelection: true)
    }

    static func dismantleNSView(_ scroll: NSScrollView, coordinator: ()) {
        (scroll.documentView as? SandboxTextView)?.field?.setViewHasKeyboard(false)
    }
}

/// The keys come through AppKit's own text input, so a key produces what it
/// would in any text field, dead keys included. Where AppKit would change the
/// text, the key goes to the core instead.
private final class SandboxTextView: NSTextView {
    var field: TestField?
    /// True while the view is being set to what the core says, so the
    /// selection that moves with it is not reported back as the user's.
    private var isDrawing = false
    private var observers: [NSObjectProtocol] = []

    /// Makes the view say what the field holds.
    ///
    /// With `keepingSelection`, a selection the user dragged out is left alone
    /// when the text has not changed under it: the core knows only where it
    /// starts, and collapsing it would make the field impossible to copy from.
    func redraw(keepingSelection: Bool = false) {
        guard let field, !hasMarkedText() else { return }
        let state = field.field
        isDrawing = true
        defer { isDrawing = false }
        let textChanged = string != state.text
        if textChanged {
            string = state.text
        }
        let range = NSRange(location: Int(state.caret), length: Int(state.selected))
        let usersOwn = keepingSelection && !textChanged && selectedRange().location == range.location
        if !usersOwn, selectedRange() != range {
            setSelectedRange(range)
        }
        scrollRangeToVisible(range)
    }

    override func insertText(_ string: Any, replacementRange: NSRange) {
        let text = (string as? NSAttributedString)?.string ?? string as? String ?? ""
        field?.type(text)
        redraw()
    }

    override func insertNewline(_ sender: Any?) {
        insertText("\n", replacementRange: NSRange(location: NSNotFound, length: 0))
    }

    override func insertTab(_ sender: Any?) {
        insertText("\t", replacementRange: NSRange(location: NSNotFound, length: 0))
    }

    override func deleteBackward(_ sender: Any?) {
        field?.type(.backspace)
        redraw()
    }

    /// Command-Z is the undo key a user presses straight after an expansion,
    /// so it goes to the core like any other key.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if window?.firstResponder === self, flags == .command,
           event.charactersIgnoringModifiers?.lowercased() == "z" {
            field?.type(.undo)
            redraw()
            return true
        }
        return super.performKeyEquivalent(with: event)
    }

    /// Anything else that would change the text (paste, cut, deleting a word)
    /// is refused: the field is the core's, and only keys reach it.
    override func shouldChangeText(in affectedCharRange: NSRange, replacementString: String?) -> Bool {
        isDrawing
    }

    override func setSelectedRanges(
        _ ranges: [NSValue], affinity: NSSelectionAffinity, stillSelecting: Bool
    ) {
        super.setSelectedRanges(ranges, affinity: affinity, stillSelecting: stillSelecting)
        guard !isDrawing, !stillSelecting, let first = ranges.first?.rangeValue else { return }
        field?.moveCaret(to: first.location)
    }

    // MARK: - The keyboard

    override func becomeFirstResponder() -> Bool {
        defer { reportKeyboard() }
        return super.becomeFirstResponder()
    }

    override func resignFirstResponder() -> Bool {
        let resigned = super.resignFirstResponder()
        field?.setViewHasKeyboard(false)
        return resigned
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
        guard let window else {
            field?.setViewHasKeyboard(false)
            return
        }
        for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification] {
            observers.append(NotificationCenter.default.addObserver(
                forName: name, object: window, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated { self?.reportKeyboard() }
            })
        }
        reportKeyboard()
    }

    /// The field has the keyboard while it is the focus of the key window: a
    /// user who has moved to another app is typing there, and the tap must be
    /// watching.
    func reportKeyboard() {
        let has = window?.isKeyWindow == true && window?.firstResponder === self
        field?.setViewHasKeyboard(has)
    }
}
