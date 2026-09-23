import AppKit
import AraloKit
import SwiftUI

/// The window the snippets live in. It opens on demand and closing it leaves
/// the agent running: the menu bar item is the app, this is one of its views.
@MainActor
final class LibraryWindowController: NSWindowController, NSWindowDelegate {
    private let store: LibraryStore

    init(store: LibraryStore, root: URL) {
        self.store = store
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 960, height: 600),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = "Aralo Snippets"
        window.setFrameAutosaveName("AraloLibrary")
        window.isReleasedWhenClosed = false
        super.init(window: window)
        var view = LibraryView(store: store, root: root)
        view.chooseImport = { [weak self] in self?.chooseImport() }
        view.export = { [weak self] format in self?.export(format) }
        window.contentViewController = NSHostingController(rootView: view)
        window.delegate = self
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used from a nib")
    }

    func show() {
        // A library that changed while the window was shut is read again, so
        // it never opens on a stale list.
        store.refresh()
        if window?.frameAutosaveName.isEmpty != false { window?.center() }
        // An agent app is never active on its own, and the editor needs keys.
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }

    /// Asks for a file from another expander, over the window, and puts the
    /// import's dry run on screen. Nothing is written until the sheet says so.
    func chooseImport() {
        show()
        guard let window else { return }
        let panel = NSOpenPanel()
        panel.title = "Import Snippets"
        panel.message = "Choose a TextExpander export, or a CSV, JSON or YAML file."
        panel.prompt = "Continue"
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            MainActor.assumeIsolated { self?.store.beginImport(of: url) }
        }
    }

    /// Writes the selected group, or the whole library, where the user says.
    private func export(_ format: ExportFormat) {
        guard let window else { return }
        let data: Data
        do {
            data = try store.export(as: format)
        } catch {
            store.attempt { _ in throw error }
            return
        }
        let panel = NSSavePanel()
        panel.title = "Export Snippets"
        panel.nameFieldStringValue = "\(store.selectedGroup.last ?? "Aralo Snippets").\(format.fileExtension)"
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            MainActor.assumeIsolated {
                self?.store.attempt { _ in try data.write(to: url, options: .atomic) }
            }
        }
    }

    /// Closing the window is not a way to throw work away: what was typed into
    /// the open snippet is written, the same as moving off it in the list.
    func windowWillClose(_ notification: Notification) {
        store.attempt { try $0.save() }
    }
}
