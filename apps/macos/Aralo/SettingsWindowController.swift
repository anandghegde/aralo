import AppKit
import AraloKit
import SwiftUI

/// Settings: AI, and where the library is. The rest of Aralo's settings join
/// them as they get panes of their own.
@MainActor
final class SettingsWindowController: NSWindowController {
    private let aiSettings: AISettingsStore
    private let location: LibraryLocationStore

    init(aiSettings: AISettingsStore, location: LibraryLocationStore) {
        self.aiSettings = aiSettings
        self.location = location
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 760, height: 580),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Aralo Settings"
        window.setFrameAutosaveName("AraloSettings")
        window.isReleasedWhenClosed = false
        super.init(window: window)
        let tabs = TabView {
            AISettingsView(store: aiSettings).tabItem { Label("AI", systemImage: "sparkles") }
            LibrarySettingsView(store: location).tabItem { Label("Library", systemImage: "folder") }
        }
        window.contentViewController = NSHostingController(rootView: tabs)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used from a nib")
    }

    func show() {
        // profiles.toml may have been edited by hand, or by `aralo ai`.
        aiSettings.refresh()
        if window?.frameAutosaveName.isEmpty != false { window?.center() }
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }
}
