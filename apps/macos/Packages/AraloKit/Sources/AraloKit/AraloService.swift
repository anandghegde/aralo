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
    private var pauseHotKey: GlobalHotKey?
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

    /// A compatibility table to use instead of the one built into the core,
    /// for measuring an app without rebuilding. Nothing a user sets.
    public static var compatTableOverride: String? {
        let path = ProcessInfo.processInfo.environment["ARALO_COMPAT"] ?? ""
        return path.isEmpty ? nil : path
    }

    /// Why the override was refused. The built-in table is in use when set.
    public private(set) var compatTableProblem: String?

    /// Who holds secure input, while `state` is `.secureInput`.
    public var secureInputHolder: SecureInputHolder? { secureInput?.holder }

    /// The shortcut that pauses and resumes from any app. Nil when the system
    /// refused it (another app owns it) or the layout has no such key.
    public private(set) var pauseShortcut: GlobalHotKey.Shortcut?

    public var snippetCount: Int { core?.snippets().count ?? 0 }
    /// The tap is up. The first run uses this rather than guessing from grants.
    public var isTapRunning: Bool { tap?.isRunning ?? false }
    /// What the first run's test field asks the user to type.
    public var onboardingExample: OnboardingExample? { core.flatMap { OnboardingExample.pick(from: $0.snippets()) } }
    public var diagnostics: [LibraryDiagnostic] { core?.diagnostics() ?? [] }
    public var isPaused: Bool { core?.engine().isPaused() ?? false }

    public func start() {
        do {
            core = try Core.openLibrary(path: libraryURL.path)
        } catch {
            state = .failed(error.localizedDescription)
            return
        }
        loadCompatTable()
        // The layout decides which key is "P", so the hot key follows it. It
        // needs no permission, so it does not wait for the tap.
        keyboardLayout.onChange = { [weak self] in self?.inputSourceChanged() }
        keyboardLayout.start()
        startTapWhenPermitted()
    }

    public func setPaused(_ paused: Bool) {
        core?.engine().setPaused(paused: paused)
        refreshState()
    }

    public func reloadLibrary() {
        do {
            try core?.reload()
            loadCompatTable()
            refreshState()
        } catch {
            state = .failed(error.localizedDescription)
        }
    }

    private func inputSourceChanged() {
        // What was typed so far was typed under other rules.
        core?.engine().reset(reason: .inputMethod)

        let shortcut = GlobalHotKey.Shortcut.pause
        let hotKey = pauseHotKey ?? GlobalHotKey { [weak self] in
            guard let self else { return }
            setPaused(!isPaused)
        }
        pauseHotKey = hotKey
        let keyCode = KeyboardLayout.currentKeyCodes(for: [shortcut.character])[shortcut.character]
        let registered = keyCode.map { hotKey.register(keyCode: $0, modifiers: shortcut.modifiers) } ?? false
        if !registered { hotKey.unregister() }
        pauseShortcut = registered ? shortcut : nil
    }

    /// A refused file changes nothing in the core, so expansion carries on
    /// with the table it had.
    private func loadCompatTable() {
        guard let core, let path = Self.compatTableOverride else { return }
        do {
            try core.loadCompatTable(path: path)
            compatTableProblem = nil
        } catch {
            compatTableProblem = error.localizedDescription
        }
    }

    private func startTapWhenPermitted() {
        guard let core, tap == nil else { return }
        let engine = core.engine()
        let sink = SystemEventSink(shortcuts: keyboardLayout.shortcuts)
        let injector = Injector(sink: sink, pasteboard: SystemPasteboard())
        let controller = ExpansionController(
            engine: engine, injector: injector, translator: keyboardLayout.translator
        )
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
        // The same clock now watches for the grant being taken away.
        permissionTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.stopTapIfRevoked() }
        }
        self.tap = tap

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

    /// macOS says nothing when a grant is revoked, and a tap left up without
    /// one can stall the keyboard. So take it down and go back to waiting.
    private func stopTapIfRevoked() {
        guard let tap, !Permissions.status.accessibility else { return }
        core?.engine().reset(reason: .manual)
        tap.stop()
        self.tap = nil
        frontApp?.stop()
        frontApp = nil
        secureInput?.stop()
        secureInput = nil
        permissionTimer?.invalidate()
        permissionTimer = nil
        startTapWhenPermitted()
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
