import AppKit
import AraloKit
import SwiftUI

/// The form panel: a snippet that asks something before it expands puts its
/// boxes here, and Enter fills them in and types the result into the app the
/// user was already in.
///
/// Like the palette, it is a non-activating panel, so the app underneath stays
/// the frontmost one and keeps its insertion point. It takes the keyboard while
/// it is up and gives it back before a single character is typed.
///
/// One panel per session: the boxes are the snippet's, and the last snippet's
/// are not this one's.
@MainActor
final class FormWindowController: NSWindowController, NSWindowDelegate {
    private let service: AraloService
    private let session: FormSession
    /// True between ordering the window out to expand and the session being
    /// over. Resigning key is what usually cancels the panel, and this is how
    /// the dismissal it does itself is told apart.
    private var isSubmitting = false

    /// Nil when no snippet is waiting to be filled in.
    init?(service: AraloService) {
        guard let session = service.form else { return nil }
        self.service = service
        self.session = session
        let panel = FormPanel(
            contentRect: NSRect(x: 0, y: 0, width: 460, height: 320),
            styleMask: [.nonactivatingPanel, .titled, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        panel.isMovableByWindowBackground = true
        panel.isReleasedWhenClosed = false
        // Above ordinary windows, and on whichever space the user is on: the
        // form belongs to the app they are typing in, not to a desktop.
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        panel.hidesOnDeactivate = false
        super.init(window: panel)
        panel.delegate = self
        panel.contentViewController = NSHostingController(
            rootView: FormView(
                session: session,
                submit: { [weak self] in self?.submit() },
                cancel: { [weak self] in self?.dismiss() }
            )
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used from a nib")
    }

    /// Shows the form over whatever the user is looking at. The app they are
    /// typing in stays active; only the keyboard moves.
    func show() {
        guard let window else { return }
        window.setFrameOrigin(Self.origin(for: window.frame.size))
        window.makeKeyAndOrderFront(nil)
        service.setFormHasKeyboard(true)
    }

    /// Fills the snippet in, out of the way.
    ///
    /// The panel goes first because synthetic keys follow the keyboard: typed
    /// with the form still up, the expansion would land in the form. Giving the
    /// keyboard back is also what arms the undo key, and what is about to land
    /// has to be undoable.
    private func submit() {
        guard !isSubmitting, !session.isFinished else { return }
        isSubmitting = true
        hide()
        session.submit()
        finish()
    }

    /// The user changed their mind. Whatever the match swallowed goes back into
    /// the document and the snippet is forgotten.
    private func dismiss() {
        guard !isSubmitting else { return }
        isSubmitting = true
        hide()
        session.cancel()
        finish()
    }

    private func hide() {
        window?.orderOut(nil)
        // Before anything is typed, never after: a reset takes the undo record
        // with it, and what is about to land has to be undoable.
        service.setFormHasKeyboard(false)
    }

    /// The panel is done with: the window goes, and so does the session it was
    /// drawing. Named apart from `NSWindowController.close()`, which it calls.
    private func finish() {
        window?.close()
        service.formClosed()
    }

    /// Clicking into another window is a way of saying never mind: an
    /// unanswered form must not sit there holding a swallowed keystroke.
    func windowDidResignKey(_ notification: Notification) {
        dismiss()
    }

    /// Where a form goes: across the screen the mouse is on, a little above the
    /// middle, where the palette opens too.
    private static func origin(for size: NSSize) -> NSPoint {
        let screen = NSScreen.screens.first { NSMouseInRect(NSEvent.mouseLocation, $0.frame, false) }
            ?? NSScreen.main
        guard let frame = screen?.visibleFrame else { return .zero }
        return NSPoint(
            x: frame.midX - size.width / 2,
            y: frame.minY + (frame.height - size.height) * 0.72
        )
    }
}

/// A panel that takes the keyboard without making Aralo the active app, so the
/// app the text is for never loses its insertion point.
private final class FormPanel: NSPanel {
    override var canBecomeKey: Bool { true }
}

/// The boxes, the preview and the buttons. The editor's test field shows the
/// same view in a sheet, so a form is tried exactly as it will be filled in.
struct FormView: View {
    let session: FormSession
    let submit: () -> Void
    let cancel: () -> Void
    /// Which box has the keyboard. The first one takes it as the panel opens,
    /// so a form can be filled in without reaching for the mouse.
    @FocusState private var focused: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            heading
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(session.fields, id: \.name) { field in
                        box(for: field)
                    }
                }
                .padding(16)
            }
            Divider()
            preview
            Divider()
            buttons
        }
        .frame(width: 460, height: 320)
        .onAppear { focused = session.fields.first?.name }
        .onExitCommand(perform: cancel)
    }

    private var heading: some View {
        Text(session.label.isEmpty ? "Fill in the snippet" : session.label)
            .font(.headline)
            .lineLimit(1)
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
    }

    /// A drop-down for a field with choices, a box with room to write in for a
    /// field that asked for lines, and one line to type in for the rest.
    @ViewBuilder
    private func box(for field: FormField) -> some View {
        let answer = binding(for: field)
        if field.hasRoomToWrite {
            VStack(alignment: .leading, spacing: 4) {
                Text(field.label).font(.callout).foregroundStyle(.secondary)
                TextEditor(text: answer)
                    .font(.body)
                    .scrollContentBackground(.hidden)
                    .padding(4)
                    .frame(height: CGFloat(field.lines) * Self.lineHeight)
                    .background(RoundedRectangle(cornerRadius: 6).fill(.background))
                    .overlay(
                        RoundedRectangle(cornerRadius: 6).strokeBorder(.separator)
                    )
                    .focused($focused, equals: field.name)
            }
        } else if field.options.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text(field.label).font(.callout).foregroundStyle(.secondary)
                TextField(field.label, text: answer)
                    .textFieldStyle(.roundedBorder)
                    .focused($focused, equals: field.name)
                    .onSubmit(submit)
            }
        } else {
            Picker(field.label, selection: answer) {
                ForEach(field.options, id: \.self) { option in
                    Text(option).tag(option)
                }
            }
            .focused($focused, equals: field.name)
        }
    }

    /// What the answers add up to. It is the expansion itself, so what is shown
    /// here is what lands in the document.
    private var preview: some View {
        ScrollView {
            Text(session.preview)
                .font(.body.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(16)
        }
        .frame(height: 96)
    }

    private var buttons: some View {
        HStack {
            if insertsWithCommand {
                Text("⌘↩ to insert").font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Button("Cancel", action: cancel).keyboardShortcut(.cancelAction)
            Button("Insert", action: submit)
                .keyboardShortcut(insertsWithCommand ? KeyboardShortcut(.return, modifiers: .command) : .defaultAction)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    /// A form with room to write in gives Return to the box, because a box
    /// that asked for lines is a box the user puts line breaks in. Command and
    /// Return insert instead, and the panel says so.
    private var insertsWithCommand: Bool {
        session.fields.contains { $0.hasRoomToWrite }
    }

    /// A line of the body font: what one line of a box stands for.
    private static let lineHeight: CGFloat = 17

    /// The answers are one dictionary, so a box binds to its own name in it.
    /// A field nobody typed into keeps the default the body gave it.
    private func binding(for field: FormField) -> Binding<String> {
        Binding(
            get: { session.answers[field.name] ?? field.initial },
            set: { session.answers[field.name] = $0 }
        )
    }
}
