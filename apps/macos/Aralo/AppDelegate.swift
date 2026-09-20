import AppKit
import AraloKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let service = AraloService(libraryURL: AraloService.defaultLibraryURL)
    private var statusMenu: StatusMenuController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        statusMenu = StatusMenuController(service: service)
        service.start()
        if !Permissions.status.accessibility {
            explainThenRequestPermissions()
        }
    }

    /// The system prompts say nothing about why. Aralo says it first.
    private func explainThenRequestPermissions() {
        let alert = NSAlert()
        alert.messageText = "Aralo needs to see what you type"
        alert.informativeText = """
            To expand an abbreviation, Aralo has to notice that you typed it. It keeps the last \
            64 characters in memory, wipes them whenever you click, switch apps or move the caret, \
            and never writes them to disk or sends them anywhere. Password fields are invisible to it.

            macOS will now ask for Accessibility and Input Monitoring access.
            """
        alert.addButton(withTitle: "Continue")
        alert.addButton(withTitle: "Not Now")
        NSApp.activate(ignoringOtherApps: true)
        if alert.runModal() == .alertFirstButtonReturn {
            Permissions.request()
        }
    }
}
