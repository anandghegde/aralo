import AppKit
import AraloKit
import SwiftUI

/// Settings: AI, and where the library is. The rest of Aralo's settings join
/// them as they get panes of their own.
@MainActor
final class SettingsWindowController: NSWindowController {
    /// A tab of Settings, for a way in that opens on the one it is about.
    enum Tab: Hashable {
        case aiProfiles
        case library
    }

    /// Which tab is showing: the window's, and whoever opens it on one.
    @Observable
    final class Selection {
        var tab: Tab = .aiProfiles
    }

    private let aiSettings: AISettingsStore
    private let location: LibraryLocationStore
    private let selection = Selection()

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
        let tabs = SettingsTabs(aiSettings: aiSettings, location: location, selection: selection)
        window.contentViewController = NSHostingController(rootView: tabs)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used from a nib")
    }

    /// Brings Settings forward, on `tab` when one is named and on whichever
    /// tab was showing when not.
    func show(tab: Tab? = nil) {
        if let tab { selection.tab = tab }
        // profiles.toml may have been edited by hand, or by `aralo ai`.
        aiSettings.refresh()
        if window?.frameAutosaveName.isEmpty != false { window?.center() }
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }
}

private struct SettingsTabs: View {
    let aiSettings: AISettingsStore
    let location: LibraryLocationStore
    @Bindable var selection: SettingsWindowController.Selection

    var body: some View {
        TabView(selection: $selection.tab) {
            AISettingsView(store: aiSettings).tabItem { Label("AI", systemImage: "sparkles") }
                .tag(SettingsWindowController.Tab.aiProfiles)
            LibrarySettingsView(store: location).tabItem { Label("Library", systemImage: "folder") }
                .tag(SettingsWindowController.Tab.library)
        }
    }
}
