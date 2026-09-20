import AppKit
import AraloKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let service = AraloService(libraryURL: AraloService.defaultLibraryURL)
    private var statusMenu: StatusMenuController?
    private var onboarding: OnboardingWindowController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let statusMenu = StatusMenuController(service: service)
        statusMenu.onSetUp = { [weak self] in self?.showOnboarding(fromMenu: true) }
        self.statusMenu = statusMenu
        service.start()
        showOnboarding(fromMenu: false)
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
