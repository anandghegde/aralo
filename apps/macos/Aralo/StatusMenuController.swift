import AppKit
import AraloKit

/// The menu bar item: state at a glance, and the few commands that exist so far.
@MainActor
final class StatusMenuController: NSObject, NSMenuDelegate {
    private let service: AraloService
    /// Opens the first-run flow, at whatever still needs doing.
    var onSetUp: (() -> Void)?
    /// Opens the window the snippets are edited in.
    var onShowLibrary: (() -> Void)?
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)

    init(service: AraloService) {
        self.service = service
        super.init()
        let menu = NSMenu()
        menu.delegate = self
        item.menu = menu
        service.onStateChange = { [weak self] _ in self?.updateIcon() }
        updateIcon()
    }

    private func updateIcon() {
        let (symbol, description) = switch service.state {
        case .active: ("text.cursor", "Aralo is on")
        case .paused: ("pause.circle", "Aralo is paused")
        case .secureInput: ("lock", "Aralo is off while a password field has the keyboard")
        case .needsPermissions: ("exclamationmark.triangle", "Aralo needs permission")
        case .failed: ("xmark.octagon", "Aralo could not open its library")
        }
        item.button?.image = NSImage(systemSymbolName: symbol, accessibilityDescription: description)
        item.button?.toolTip = description
    }

    /// Rebuilt each time it opens, so it never shows a stale state.
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        let top = disabled(headline)
        if service.state == .secureInput {
            top.toolTip = "macOS hides every key from Aralo while an app has secure input on. "
                + "If no password field is open, quitting that app turns it off."
        }
        menu.addItem(top)

        switch service.state {
        case .active, .paused, .secureInput:
            let title = service.isPaused ? "Resume Aralo" : "Pause Aralo"
            let pause = command(title, #selector(togglePause))
            if let shortcut = service.pauseShortcut {
                // Shown for the user to learn. The hot key itself is global.
                pause.keyEquivalent = String(shortcut.character)
                pause.keyEquivalentModifierMask = shortcut.modifiers
            }
            menu.addItem(pause)
        case .needsPermissions:
            menu.addItem(command("Set Up Aralo…", #selector(setUp)))
        case .failed:
            break
        }

        menu.addItem(.separator())
        let problems = service.diagnostics.filter { $0.level == .error }
        if !problems.isEmpty {
            let files = problems.count == 1 ? "1 file" : "\(problems.count) files"
            let entry = disabled("\(files) could not be loaded")
            entry.toolTip = problems.map { "\($0.path): \($0.message)" }.joined(separator: "\n")
            menu.addItem(entry)
        }
        if let problem = service.compatTableProblem {
            let entry = disabled("Compatibility table override not loaded")
            entry.toolTip = problem
            menu.addItem(entry)
        }
        let snippets = command("Snippets\u{2026}", #selector(showLibrary))
        snippets.keyEquivalent = ","
        snippets.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(snippets)
        menu.addItem(command("Open Library Folder", #selector(openLibrary)))
        menu.addItem(command("Reload Library", #selector(reloadLibrary)))
        menu.addItem(.separator())
        let quit = #selector(NSApplication.terminate(_:))
        menu.addItem(NSMenuItem(title: "Quit Aralo", action: quit, keyEquivalent: "q"))
    }

    private var headline: String {
        switch service.state {
        case .active:
            let count = service.snippetCount
            return count == 1 ? "On · 1 snippet" : "On · \(count) snippets"
        case .paused: return "Paused"
        case .secureInput:
            guard let holder = service.secureInputHolder else {
                return "Off while a password field has the keyboard"
            }
            return "Off while \(holder.name ?? "process \(holder.pid)") has secure input on"
        case .needsPermissions: return "Waiting for permission"
        case .failed(let message): return "Library problem: \(message)"
        }
    }

    private func disabled(_ title: String) -> NSMenuItem {
        let entry = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        entry.isEnabled = false
        return entry
    }

    private func command(_ title: String, _ action: Selector) -> NSMenuItem {
        let entry = NSMenuItem(title: title, action: action, keyEquivalent: "")
        entry.target = self
        return entry
    }

    @objc private func togglePause() { service.setPaused(!service.isPaused) }
    @objc private func reloadLibrary() { service.reloadLibrary() }
    @objc private func openLibrary() { NSWorkspace.shared.open(service.libraryURL) }
    @objc private func setUp() { onSetUp?() }
    @objc private func showLibrary() { onShowLibrary?() }
}
