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

    func applicationDidFinishLaunching(_ notification: Notification) {
        let statusMenu = StatusMenuController(service: service)
        statusMenu.onSetUp = { [weak self] in self?.showOnboarding(fromMenu: true) }
        statusMenu.onShowLibrary = { [weak self] in self?.showLibrary() }
        statusMenu.onSearchSnippets = { [weak self] in self?.service.openPalette() }
        statusMenu.onImport = { [weak self] in self?.libraryWindow()?.chooseImport() }
        self.statusMenu = statusMenu
        service.onPaletteRequested = { [weak self] in self?.showPalette() }
        service.onFormRequested = { [weak self] in self?.showForm() }
        service.start()
        showOnboarding(fromMenu: false)
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
