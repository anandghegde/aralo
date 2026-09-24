import AppKit
import AraloKit
import SwiftUI

/// Settings. It has one tab so far, AI; the rest of Aralo's settings join it
/// as they get panes of their own.
@MainActor
final class SettingsWindowController: NSWindowController {
    private let aiSettings: AISettingsStore

    init(aiSettings: AISettingsStore) {
        self.aiSettings = aiSettings
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
