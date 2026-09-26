import AppKit
import AraloKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let service = AraloService(libraryURL: AraloService.defaultLibraryURL)
    private var statusMenu: StatusMenuController?
    private var onboarding: OnboardingWindowController?
    private var library: LibraryWindowController?
    private var palette: PaletteWindowController?
    private var form: FormWindowController?
    private var commands: CommandWindowController?
    private var settings: SettingsWindowController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let statusMenu = StatusMenuController(service: service)
        statusMenu.onSetUp = { [weak self] in self?.showOnboarding(fromMenu: true) }
        statusMenu.onShowLibrary = { [weak self] in self?.showLibrary() }
        statusMenu.onSearchSnippets = { [weak self] in self?.service.openPalette() }
        statusMenu.onTransformSelection = { [weak self] in self?.service.openCommands() }
        statusMenu.onImport = { [weak self] in self?.libraryWindow()?.chooseImport() }
        statusMenu.onShowSettings = { [weak self] in self?.showSettings() }
        self.statusMenu = statusMenu
        service.onPaletteRequested = { [weak self] in self?.showPalette() }
        service.onFormRequested = { [weak self] in self?.showForm() }
        service.onCommandsRequested = { [weak self] in self?.showCommands() }
        // The snippet window shows the folder it was made for; the next one
        // shows where the library went.
        service.onLibraryMoved = { [weak self] _ in
            self?.library?.window?.close()
            self?.library = nil
        }
        service.start()
        showOnboarding(fromMenu: false)
    }

    /// The counters are saved a minute after they change; this saves the
    /// last minute.
    func applicationWillTerminate(_ notification: Notification) {
        service.saveCounters()
    }

    /// The snippet window, made the first time it is asked for. It is kept
    /// afterwards so that reopening it lands on the same selection.
    private func showLibrary() {
        libraryWindow()?.show()
    }

    private func libraryWindow() -> LibraryWindowController? {
        guard let store = service.library else { return nil }
        let controller = library ?? LibraryWindowController(store: store, root: service.libraryURL)
        library = controller
        return controller
    }

    /// Settings, made the first time they are asked for. The AI settings open
    /// then too: until that, nothing reads profiles.toml or the keychain.
    private func showSettings() {
        if settings == nil {
            do {
                // The service's instance, which the command panel runs with.
                settings = SettingsWindowController(
                    aiSettings: AISettingsStore(settings: try service.aiSettings()),
                    location: LibraryLocationStore(service: service)
                )
            } catch {
                let alert = NSAlert()
                alert.messageText = "Aralo could not open its AI settings"
                alert.informativeText = error.localizedDescription
                alert.runModal()
                return
            }
        }
        settings?.show()
    }

    /// The search palette, made the first time the hot key is pressed. It is
    /// kept afterwards: it opens often, and on the key a user is still holding.
    private func showPalette() {
        let controller = palette ?? PaletteWindowController(service: service)
        palette = controller
        controller?.show()
    }

    /// The form panel, made fresh for each snippet that asks something: the
    /// boxes are that snippet's, and the last one's are not this one's.
    private func showForm() {
        form?.window?.close()
        let controller = FormWindowController(service: service)
        form = controller
        controller?.show()
    }

    /// The command panel, made fresh for each selection: the text it works on
    /// is the one read when the hot key was pressed.
    private func showCommands() {
        commands?.window?.close()
        let controller = CommandWindowController(service: service)
        commands = controller
        controller?.show()
    }

    /// The first run, or the part of it that a revoked permission calls for.
    /// The system prompts say nothing about why; this says it first. From the
    /// menu it always opens, so the test field stays reachable.
    private func showOnboarding(fromMenu: Bool) {
        let completed = UserDefaults.standard.bool(forKey: OnboardingWindowController.completedKey)
        let facts = OnboardingFlow.Facts(permissions: Permissions.status, tapRunning: service.isTapRunning)
        let entry = OnboardingFlow.entryPoint(completedBefore: completed, facts: facts)
        guard let step = entry ?? (fromMenu ? .tryIt : nil) else { return }
        onboarding?.close()
        let controller = OnboardingWindowController(service: service, startingAt: step)
        onboarding = controller
        controller.show()
    }
}
