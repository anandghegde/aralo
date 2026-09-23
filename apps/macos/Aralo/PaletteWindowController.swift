import AppKit
import AraloKit
import Observation
import SwiftUI

/// The search palette: a hot key anywhere, a few letters, and Enter puts the
/// snippet into the app the user was already in.
///
/// The window is a non-activating panel, so the app underneath stays the
/// frontmost one and keeps its insertion point. It takes the keyboard while it
/// is up and gives it back before a single character is typed.
@MainActor
final class PaletteWindowController: NSWindowController, NSWindowDelegate {
    private let service: AraloService
    private let store: PaletteStore
    /// True between ordering the window out to insert and the insertion
    /// answering. Resigning key is what usually dismisses the palette, and
    /// this is how the dismissal it does itself is told apart.
    private var isInserting = false
    /// Bumped every time the palette is shown, so the search box takes the
    /// keyboard again. The window is only ordered out between uses, so nothing
    /// in the view appears or disappears to notice it on its own.
    private let focus = PaletteFocus()

    /// Nil before Aralo may type, which is when there is no palette to show.
    init?(service: AraloService) {
        guard let store = service.palette else { return nil }
        self.service = service
        self.store = store
        let panel = PalettePanel(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 400),
            styleMask: [.nonactivatingPanel, .titled, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        panel.isMovableByWindowBackground = true
        panel.isReleasedWhenClosed = false
        // Above ordinary windows, and on whichever space the user is on: the
        // palette belongs to the app they are typing in, not to a desktop.
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        // An agent app never deactivates, so this is about the panel only: it
        // goes when it loses the keyboard, and `windowDidResignKey` says so.
        panel.hidesOnDeactivate = false
        super.init(window: panel)
        panel.delegate = self
        panel.contentViewController = NSHostingController(
            rootView: PaletteView(
                store: store,
                focus: focus,
                insert: { [weak self] in self?.insertSelected() },
                cancel: { [weak self] in self?.dismiss() }
            )
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used from a nib")
    }

    /// Shows the palette over whatever the user is looking at. The app they
    /// came from stays active; only the keyboard moves.
    func show() {
        guard let window else { return }
        window.setFrameOrigin(Self.origin(for: window.frame.size))
        window.makeKeyAndOrderFront(nil)
        focus.take()
        service.setPaletteHasKeyboard(true)
    }

    /// Takes the palette away for good: the app is the user's again, and the
    /// snippet they did not pick is forgotten.
    private func dismiss() {
        hide()
        service.paletteClosed()
    }

    private func hide() {
        window?.orderOut(nil)
        // Before anything is typed, never after: a reset takes the undo record
        // with it, and what is about to land has to be undoable.
        service.setPaletteHasKeyboard(false)
    }

    /// Inserts what is selected, out of the way.
    ///
    /// The panel goes first because synthetic keys follow the keyboard: typed
    /// with the palette still up, the snippet would land in the search box. If
    /// nothing goes in, the palette comes back to say why, which is the only
    /// place a user ever hears about a refusal.
    private func insertSelected() {
        guard store.selection != nil, !isInserting else { return }
        isInserting = true
        hide()
        Task { @MainActor in
            let inserted = await store.insertSelected()
            isInserting = false
            if inserted {
                service.paletteClosed()
            } else {
                show()
            }
        }
    }

    /// Clicking into another window is a way of saying never mind. The panel
    /// is the only window Aralo has that the keyboard can leave while it is
    /// still on screen.
    func windowDidResignKey(_ notification: Notification) {
        guard !isInserting else { return }
        dismiss()
    }

    /// Where a palette goes: across the screen the mouse is on, a little above
    /// the middle, which is where every picker on the Mac opens.
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
private final class PalettePanel: NSPanel {
    override var canBecomeKey: Bool { true }
}

/// Says when the search box should take the keyboard. A count rather than a
/// flag: showing the palette twice has to say it twice.
@MainActor
@Observable
private final class PaletteFocus {
    private(set) var generation = 0

    func take() {
        generation += 1
    }
}

private struct PaletteView: View {
    @Bindable var store: PaletteStore
    let focus: PaletteFocus
    let insert: () -> Void
    let cancel: () -> Void
    @FocusState private var searching: Bool

    var body: some View {
        VStack(spacing: 0) {
            search
            Divider()
            if store.rows.isEmpty {
                empty
            } else {
                HSplitView {
                    list
                    preview
                }
            }
            if let refusal = store.refusal {
                Divider()
                Label(refusal, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(width: 640, height: 400)
        .onExitCommand(perform: cancel)
    }

    private var search: some View {
        HStack(spacing: 10) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("Search snippets", text: $store.query)
                .textFieldStyle(.plain)
                .font(.title2)
                .focused($searching)
                .onSubmit(insert)
                // The list is what the arrows move through, not the cursor:
                // the search box is one line and the user is picking a row.
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
        .onChange(of: focus.generation) { searching = true }
    }

    private var list: some View {
        ScrollViewReader { scroll in
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(store.rows) { row in
                        PaletteRow(row: row, isSelected: row.id == store.selection)
                            .id(row.id)
                            .contentShape(.rect)
                            .onTapGesture(count: 2, perform: insert)
                            .onTapGesture { store.select(row.id) }
                    }
                }
                .padding(.vertical, 6)
            }
            .onChange(of: store.selection) { _, selection in
                guard let selection else { return }
                withAnimation(.linear(duration: 0.1)) { scroll.scrollTo(selection) }
            }
        }
        .frame(minWidth: 240, idealWidth: 300)
    }

    private var preview: some View {
        ScrollView {
            Text(store.preview)
                .font(.body.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(16)
        }
        .frame(minWidth: 220)
    }

    private var empty: some View {
        VStack(spacing: 6) {
            Text("No snippets match").font(.title3)
            Text("Press Escape to go back to what you were doing.")
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct PaletteRow: View {
    let row: SnippetRow
    let isSelected: Bool

    var body: some View {
        HStack(spacing: 8) {
            Text(row.label).lineLimit(1)
            Spacer(minLength: 8)
            if let abbreviation = row.abbreviations.first {
                Text(abbreviation)
                    .font(.callout.monospaced())
                    .foregroundStyle(isSelected ? .primary : .secondary)
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .background(isSelected ? Color.accentColor.opacity(0.85) : .clear, in: .rect(cornerRadius: 6))
        .foregroundStyle(isSelected ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
        .padding(.horizontal, 8)
    }
}
