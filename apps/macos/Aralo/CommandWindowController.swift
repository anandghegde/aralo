import AppKit
import AraloBridge
import AraloKit
import SwiftUI

/// The command panel (PRD A3): pick a command, read the answer against the
/// selection word by word, and Enter puts it in place of the selection.
///
/// Like the palette, it is a non-activating panel, so the app underneath stays
/// the frontmost one and keeps its selection. It takes the keyboard while it is
/// up and gives it back before the answer is pasted.
///
/// One panel per selection: the model holds the text it was opened for.
@MainActor
final class CommandWindowController: NSWindowController, NSWindowDelegate {
    private let service: AraloService
    private let store: CommandStore
    /// True between ordering the window out to replace and the replacement
    /// answering, so the dismissal it does itself is not taken for the user's.
    private var isReplacing = false
    private var isFinished = false

    /// Nil when there is no command panel to show.
    init?(service: AraloService) {
        guard let store = service.commands else { return nil }
        self.service = service
        self.store = store
        let panel = CommandPanel(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 440),
            styleMask: [.nonactivatingPanel, .titled, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        panel.isMovableByWindowBackground = true
        panel.isReleasedWhenClosed = false
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        panel.hidesOnDeactivate = false
        super.init(window: panel)
        panel.delegate = self
        panel.contentViewController = NSHostingController(
            rootView: CommandView(
                store: store,
                replace: { [weak self] in self?.replace() },
                escape: { [weak self] in self?.escape() }
            )
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used from a nib")
    }

    func show() {
        guard let window else { return }
        window.setFrameOrigin(Self.origin(for: window.frame.size))
        window.makeKeyAndOrderFront(nil)
        service.setCommandsHaveKeyboard(true)
    }

    /// Replaces the selection, out of the way: the paste goes wherever the
    /// keyboard is. If nothing goes in, the panel comes back to say why.
    private func replace() {
        guard store.phase == .answered, !isReplacing else { return }
        isReplacing = true
        hide()
        Task { @MainActor in
            let replaced = await store.replace()
            isReplacing = false
            if replaced {
                finish()
            } else {
                show()
            }
        }
    }

    /// Escape stops a command that is running and goes back to the list;
    /// otherwise it closes the panel, and the selection is left as it was.
    private func escape() {
        if store.phase == .running {
            store.cancel()
        } else {
            dismiss()
        }
    }

    private func dismiss() {
        guard !isReplacing else { return }
        hide()
        finish()
    }

    private func hide() {
        window?.orderOut(nil)
        service.setCommandsHaveKeyboard(false)
    }

    private func finish() {
        guard !isFinished else { return }
        isFinished = true
        window?.close()
        service.commandsClosed()
    }

    /// Clicking into another window is a way of saying never mind.
    func windowDidResignKey(_ notification: Notification) {
        guard !isReplacing else { return }
        dismiss()
    }

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

private final class CommandPanel: NSPanel {
    override var canBecomeKey: Bool { true }
}

struct CommandView: View {
    @Bindable var store: CommandStore
    let replace: () -> Void
    let escape: () -> Void
    @FocusState private var searching: Bool
    @FocusState private var editing: Bool

    var body: some View {
        VStack(spacing: 0) {
            if let problem = store.problem {
                message(problem, detail: "Press Escape to go back to what you were doing.")
            } else if store.phase == .choosing {
                search
                Divider()
                list
                Divider()
                selected
            } else {
                header
                Divider()
                answer
                Divider()
                footer
            }
        }
        .frame(width: 640, height: 440)
        .onExitCommand(perform: escape)
    }

    // MARK: Choosing

    private var search: some View {
        HStack(spacing: 10) {
            Image(systemName: "sparkles").foregroundStyle(.secondary).accessibilityHidden(true)
            TextField("Transform the selection", text: $store.query)
                .textFieldStyle(.plain)
                .font(.title2)
                .focused($searching)
                .onSubmit { store.runHighlighted() }
                .onKeyPress(.upArrow) {
                    store.selectPrevious()
                    return .handled
                }
                .onKeyPress(.downArrow) {
                    store.selectNext()
                    return .handled
                }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .onAppear { searching = true }
    }

    private var list: some View {
        ScrollView {
            LazyVStack(spacing: 0) {
                ForEach(store.rows, id: \.id) { command in
                    CommandRow(command: command, isSelected: command.id == store.highlighted)
                        .contentShape(.rect)
                        // VoiceOver reads the row as one button, and pressing
                        // it runs the command as a double click would.
                        .accessibilityAction {
                            store.select(command.id)
                            store.runHighlighted()
                        }
                        .onTapGesture(count: 2) {
                            store.select(command.id)
                            store.runHighlighted()
                        }
                        .onTapGesture { store.select(command.id) }
                }
            }
            .padding(.vertical, 6)
        }
        .frame(maxHeight: .infinity)
    }

    private var selected: some View {
        Text(store.selection ?? "")
            .font(.callout)
            .foregroundStyle(.secondary)
            .lineLimit(2)
            .truncationMode(.tail)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
    }

    // MARK: Running and answered

    private var header: some View {
        HStack(spacing: 8) {
            Text(store.running?.label ?? "").font(.title3.weight(.semibold))
            Spacer()
            if store.phase == .running {
                ProgressView().controlSize(.small).accessibilityLabel("Running")
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    @ViewBuilder private var answer: some View {
        switch store.phase {
        case .failed(let reason):
            message(reason, detail: "Press R to try again, or Escape to close.")
        case .answered where store.isEditing:
            TextEditor(text: $store.draft)
                .accessibilityLabel("Answer")
                .font(.body)
                .focused($editing)
                .padding(12)
                .onAppear { editing = true }
        case .answered:
            ScrollView {
                Text(DiffText.render(store.diff))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
            }
        default:
            ScrollView {
                Text(store.streamed)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
            }
        }
    }

    private var footer: some View {
        VStack(alignment: .leading, spacing: 8) {
            if store.cutShort {
                Label("The model stopped before it finished. The end may be missing.",
                      systemImage: "exclamationmark.triangle")
                    .font(.callout)
            }
            if let refusal = store.refusal {
                Label(refusal, systemImage: "exclamationmark.triangle").font(.callout)
            }
            HStack(spacing: 8) {
                ManifestLabel(ContextManifest(
                    store.sent, profile: store.profile, model: store.model, request: "the instruction"
                ))
                Spacer()
                if store.phase == .answered {
                    if !store.isEditing {
                        Button("Edit", action: store.edit).keyboardShortcut("e", modifiers: [])
                        Button("Regenerate", action: store.regenerate).keyboardShortcut("r", modifiers: [])
                        Button("Replace", action: replace).keyboardShortcut(.defaultAction)
                    } else {
                        // Return makes a new line in the editor.
                        Button("Replace", action: replace).keyboardShortcut(.return, modifiers: .command)
                    }
                } else if case .failed = store.phase, store.running != nil {
                    Button("Try Again", action: store.regenerate).keyboardShortcut("r", modifiers: [])
                } else {
                    Button("Stop", action: store.cancel)
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    /// What went where, the privacy promise in one line: which context the
    /// request carried, and which profile and model it went to.
    private func message(_ text: String, detail: String) -> some View {
        VStack(spacing: 6) {
            Text(text).font(.title3).multilineTextAlignment(.center)
            Text(detail).foregroundStyle(.secondary)
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct CommandRow: View {
    let command: AiCommand
    let isSelected: Bool
    @Environment(\.colorSchemeContrast) private var contrast

    var body: some View {
        HStack(spacing: 8) {
            Text(command.label).lineLimit(1)
            Spacer(minLength: 8)
            if !command.builtin {
                Text("Library")
                    .font(.caption)
                    .foregroundStyle(isSelected ? .primary : .secondary)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .background(isSelected ? SelectedRow.fill(contrast) : .clear, in: .rect(cornerRadius: 6))
        .foregroundStyle(isSelected ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
        .padding(.horizontal, 8)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
    }
}

/// The highlight behind the chosen row of the palette and the command panel.
/// With Increase Contrast on it is solid, so white text on it keeps its
/// contrast whatever is behind the panel.
enum SelectedRow {
    static func fill(_ contrast: ColorSchemeContrast) -> Color {
        contrast == .increased ? .accentColor : .accentColor.opacity(0.85)
    }
}

/// A word diff as text: struck-through red for what goes, green for what comes
/// in. The command panel and the editor's AI sheet both draw it.
enum DiffText {
    static func render(_ spans: [DiffSpan]) -> AttributedString {
        var text = AttributedString()
        for span in spans {
            var piece = AttributedString(span.text)
            switch span.change {
            case .same:
                break
            case .removed:
                piece.strikethroughStyle = .single
                piece.foregroundColor = .red
                piece.backgroundColor = .red.opacity(0.12)
            case .added:
                piece.foregroundColor = .green
                piece.backgroundColor = .green.opacity(0.12)
            }
            text += piece
        }
        return text
    }
}
