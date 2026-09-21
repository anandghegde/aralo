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
        window.contentViewController = NSHostingController(rootView: LibraryView(store: store, root: root))
        super.init(window: window)
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

    /// Closing the window is not a way to throw work away: what was typed into
    /// the open snippet is written, the same as moving off it in the list.
    func windowWillClose(_ notification: Notification) {
        store.attempt { try $0.save() }
    }
}
