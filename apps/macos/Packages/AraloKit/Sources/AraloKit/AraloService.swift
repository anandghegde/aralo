import AraloBridge
import Foundation

/// Everything the app needs, wired together: the open library, the tap, the
/// injector and the monitors. The app's windows and menus only talk to this.
@MainActor
public final class AraloService {
    public enum State: Equatable, Sendable {
        case active
        case paused
        case secureInput
        case needsPermissions
        case failed(String)
    }

    public private(set) var state: State = .needsPermissions {
        didSet {
            if state != oldValue { onStateChange?(state) }
        }
    }

    public var onStateChange: (@MainActor (State) -> Void)?

    public let libraryURL: URL
    private var core: Core?
    private var tap: EventTap?
    private var secureInput: SecureInputMonitor?
    private var frontApp: FrontAppMonitor?
    private let keyboardLayout = KeyboardLayoutMonitor()
    private var permissionTimer: Timer?

    public init(libraryURL: URL) {
        self.libraryURL = libraryURL
    }

    /// Where the library lives unless the user chose somewhere else: a visible
    /// folder that is easy to sync, outside the folders macOS guards with
    /// their own permission prompts.
    public static var defaultLibraryURL: URL {
        if let override = ProcessInfo.processInfo.environment["ARALO_LIBRARY"], !override.isEmpty {
            return URL(fileURLWithPath: override, isDirectory: true)
        }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Aralo", isDirectory: true)
    }

    public var snippetCount: Int { core?.snippets().count ?? 0 }
    public var diagnostics: [LibraryDiagnostic] { core?.diagnostics() ?? [] }
    public var isPaused: Bool { core?.engine().isPaused() ?? false }

    public func start() {
        do {
            core = try Core.openLibrary(path: libraryURL.path)
        } catch {
            state = .failed(error.localizedDescription)
            return
        }
        startTapWhenPermitted()
    }

    public func setPaused(_ paused: Bool) {
        core?.engine().setPaused(paused: paused)
        refreshState()
    }

    public func reloadLibrary() {
        do {
            try core?.reload()
            refreshState()
        } catch {
            state = .failed(error.localizedDescription)
        }
    }

    private func startTapWhenPermitted() {
        guard let core, tap == nil else { return }
        let engine = core.engine()
        let sink = SystemEventSink(shortcuts: keyboardLayout.shortcuts)
        let injector = Injector(sink: sink, pasteboard: SystemPasteboard())
        let controller = ExpansionController(engine: engine, injector: injector)
        let tap = EventTap { controller.handle($0) }
        // Creating the tap is the real test. macOS refuses it until the user
        // has granted access, and gives no callback when they do, so keep asking.
        guard Permissions.status.accessibility, (try? tap.start()) != nil else {
            state = .needsPermissions
            if permissionTimer == nil {
                permissionTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
                    MainActor.assumeIsolated { self?.startTapWhenPermitted() }
                }
            }
            return
        }
        permissionTimer?.invalidate()
        permissionTimer = nil
        self.tap = tap
        keyboardLayout.start()

        let frontApp = FrontAppMonitor { engine.setFrontApp(bundleId: $0) }
        frontApp.start()
        self.frontApp = frontApp

        let secureInput = SecureInputMonitor { [weak self] isOn in
            if isOn { engine.reset(reason: .secureInput) }
            self?.refreshState()
        }
        secureInput.start()
        self.secureInput = secureInput
        refreshState()
    }

    private func refreshState() {
        if case .failed = state, core == nil { return }
        if tap == nil {
            state = .needsPermissions
        } else if secureInput?.isSecureInputOn == true {
            state = .secureInput
        } else {
            state = isPaused ? .paused : .active
        }
    }
}
