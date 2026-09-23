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

    /// The library changed: someone edited a file, or the app itself saved
    /// one. A window that lists snippets redraws from this rather than asking
    /// on a timer.
    public var onLibraryChange: (@MainActor (LibraryEvent) -> Void)?

    /// The main window's model, once the library is open. Nil while the core
    /// has not loaded, which is the only state with no library to show.
    public private(set) var library: LibraryStore?

    /// The search palette's model, once Aralo is allowed to type. Nil before
    /// that: a picker that cannot insert what is picked has nothing to offer.
    public private(set) var palette: PaletteStore?

    /// The palette's hot key was pressed and the palette is ready to be shown.
    /// The window is the shell's business; everything else has happened
    /// already.
    public var onPaletteRequested: (@MainActor () -> Void)?

    /// The form panel's model, while a snippet that asks something is waiting
    /// for an answer. Nil the rest of the time, which is nearly always.
    public private(set) var form: FormSession?

    /// A snippet asked something before expanding, and the panel is ready to
    /// be shown. On the same terms as `onPaletteRequested`: the window is the
    /// shell's business.
    public var onFormRequested: (@MainActor () -> Void)?

    public let libraryURL: URL
    public let cacheURL: URL
    private var core: Core?
    private var tap: EventTap?
    private var controller: ExpansionController?
    private var secureInput: SecureInputMonitor?
    private var frontApp: FrontAppMonitor?
    private let keyboardLayout = KeyboardLayoutMonitor()
    /// One pasteboard for the whole app: the injector pastes with it, and a
    /// `{{clipboard}}` placeholder reads it.
    private let pasteboard = SystemPasteboard()
    private var pauseHotKey: GlobalHotKey?
    private var paletteHotKey: GlobalHotKey?
    /// Which of Aralo's own windows have the keyboard. A set rather than a
    /// flag: a form panel opens from the palette, and the palette saying it
    /// has gone must not answer for the panel that is still up.
    private var keyboardOwners: Set<KeyboardOwner> = []
    /// The app the open palette will insert into: whatever was in front when
    /// the hot key was pressed, held until the palette closes.
    private var paletteTarget: TargetApp?
    private var permissionTimer: Timer?

    public init(libraryURL: URL, cacheURL: URL = AraloService.defaultCacheURL) {
        self.libraryURL = libraryURL
        self.cacheURL = cacheURL
    }

    /// Why the override was refused. The built-in table is in use when set.
    public private(set) var compatTableProblem: String?

    /// Why the last read of the folder failed, when one did. The library that
    /// was loaded stays in use, so this is something to show, not a stop.
    public private(set) var libraryProblem: String?

    /// Who holds secure input, while `state` is `.secureInput`.
    public var secureInputHolder: SecureInputHolder? { secureInput?.holder }

    /// The shortcut that pauses and resumes from any app. Nil when the system
    /// refused it (another app owns it) or the layout has no such key.
    public private(set) var pauseShortcut: GlobalHotKey.Shortcut?

    /// The shortcut that opens the search palette from any app, on the same
    /// terms as `pauseShortcut`.
    public private(set) var paletteShortcut: GlobalHotKey.Shortcut?

    public var snippetCount: Int { core?.snippets().count ?? 0 }
    /// The tap is up. The first run uses this rather than guessing from grants.
    public var isTapRunning: Bool { tap?.isRunning ?? false }
    /// What the first run's test field asks the user to type.
    public var onboardingExample: OnboardingExample? { core.flatMap { OnboardingExample.pick(from: $0.snippets()) } }
    public var diagnostics: [LibraryDiagnostic] { core?.diagnostics() ?? [] }
    public var isPaused: Bool { core?.engine().isPaused() ?? false }

    public func start() {
        let watcher = LibraryWatcher { [weak self] event in
            Task { @MainActor in self?.libraryChanged(event) }
        }
        do {
            let trash = SystemTrash()
            let core = try Core.openLibrary(path: libraryURL.path, cache: cacheURL.path, events: watcher, trash: trash)
            // How a date is written is the Mac's business, not the core's, so
            // the shell says which locale it is in. Without this the core
            // falls back to the environment and then to en_US (ADR-0014).
            core.setLocale(tag: Locale.current.identifier)
            self.core = core
            library = LibraryStore(core: core) { [weak self] in self?.setKeyboard(.testField, $0) }
        } catch {
            state = .failed(error.reason)
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
            library?.refresh()
            loadCompatTable()
            refreshState()
        } catch {
            state = .failed(error.reason)
        }
    }

    /// The core watches the folder, so a reload is only for a user who would
    /// rather be sure, or for a folder that arrived while Aralo was not
    /// looking.
    private func libraryChanged(_ event: LibraryEvent) {
        if case .failed(let message) = event {
            libraryProblem = message
        } else {
            libraryProblem = nil
        }
        // The window's model first, so that a menu or a view woken by
        // `onLibraryChange` reads a list that already has the change in it.
        library?.libraryChanged(event)
        onLibraryChange?(event)
    }

    private func inputSourceChanged() {
        // What was typed so far was typed under other rules.
        core?.engine().reset(reason: .inputMethod)

        let pause = pauseHotKey ?? GlobalHotKey { [weak self] in
            guard let self else { return }
            setPaused(!isPaused)
        }
        pauseHotKey = pause
        pauseShortcut = register(.pause, on: pause)

        let palette = paletteHotKey ?? GlobalHotKey { [weak self] in self?.openPalette() }
        paletteHotKey = palette
        paletteShortcut = register(.palette, on: palette)
    }

    /// Points a hot key at the key the shortcut names on the layout in use, and
    /// returns the shortcut when the system allowed it. Nil leaves nothing
    /// registered, so a shortcut another app owns does not half-work.
    private func register(_ shortcut: GlobalHotKey.Shortcut, on hotKey: GlobalHotKey) -> GlobalHotKey.Shortcut? {
        let keyCode = shortcut.fixedKeyCode
            ?? KeyboardLayout.currentKeyCodes(for: [shortcut.character])[shortcut.character]
        let registered = keyCode.map { hotKey.register(keyCode: $0, modifiers: shortcut.modifiers) } ?? false
        if !registered { hotKey.unregister() }
        return registered ? shortcut : nil
    }

    // MARK: - The palette

    /// Opens the search palette over the app in front: notes the app the text
    /// will go into, empties the search box, and asks the shell for a window.
    ///
    /// Does nothing before Aralo may type. The hot key needs no permission and
    /// so answers sooner than the rest of the app works; a palette that could
    /// only say "nothing was inserted" is worse than no palette.
    public func openPalette() {
        guard let palette, controller != nil else { return }
        paletteTarget = RunningTargetApp.frontmost()
        palette.reset()
        onPaletteRequested?()
    }

    /// Whether the palette's window has the keyboard.
    ///
    /// While it does, Aralo watches nothing: what is typed into the search box
    /// is a query, not text in a document, and an abbreviation typed there must
    /// not expand into a field that is about to close. The buffer is emptied on
    /// the way in and on the way out, so the two sides of the palette cannot
    /// join up into an abbreviation neither of them typed.
    ///
    /// The window gives the keyboard back *before* a snippet is inserted: what
    /// clears the buffer clears the undo record with it, and the snippet that
    /// is about to land has to be undoable.
    public func setPaletteHasKeyboard(_ hasKeyboard: Bool) {
        setKeyboard(.palette, hasKeyboard)
    }

    /// The palette has gone for good. The app it was opened over is no longer
    /// spoken for, so a plan is never run against an app the user has since
    /// left.
    public func paletteClosed() {
        paletteTarget = nil
        setKeyboard(.palette, false)
    }

    // MARK: - The form panel

    /// Whether the form panel has the keyboard, on the palette's terms: while
    /// it does, what is typed is an answer to a question, not text in a
    /// document.
    ///
    /// The panel gives the keyboard back *before* it submits, for the same
    /// reason the palette does: what clears the buffer clears the undo record
    /// with it, and the expansion that is about to land has to be undoable.
    public func setFormHasKeyboard(_ hasKeyboard: Bool) {
        setKeyboard(.form, hasKeyboard)
    }

    /// The panel has gone, submitted or cancelled. Either way the session is
    /// over and the keyboard is the user's again.
    public func formClosed() {
        form = nil
        setKeyboard(.form, false)
    }

    /// A snippet that asks something before it expands. The session is the
    /// core's; this is the model a panel draws.
    ///
    /// Nothing is shown for a session with nothing to ask: it has run by the
    /// time the model is built, which is how a body that only wants the
    /// clipboard expands without a panel ever appearing.
    private func sessionStarted(_ session: ExpansionSession) {
        guard let controller else {
            // The tap went down between the keystroke and this. There is no
            // injector to put anything back with, so the session is dropped
            // rather than left open.
            _ = session.cancel()
            return
        }
        let form = FormSession(
            session: session,
            runner: controller,
            label: core?.snippet(id: session.snippetId())?.draft.label ?? "",
            clipboard: { [pasteboard] in pasteboard.text() }
        )
        guard !form.isFinished else { return }
        self.form = form
        onFormRequested?()
    }

    /// The tap watches nothing while any window of Aralo's own holds the
    /// keyboard. The buffer is emptied on the way in and on the way out, so
    /// the two sides of a panel cannot join up into an abbreviation neither of
    /// them typed.
    private func setKeyboard(_ owner: KeyboardOwner, _ hasKeyboard: Bool) {
        if hasKeyboard {
            keyboardOwners.insert(owner)
        } else {
            keyboardOwners.remove(owner)
        }
        controller?.setSuspended(!keyboardOwners.isEmpty)
    }

    /// A refused file changes nothing in the core, so expansion carries on
    /// with the table it had.
    private func loadCompatTable() {
        guard let core, let path = Self.compatTableOverride else { return }
        do {
            try core.loadCompatTable(path: path)
            compatTableProblem = nil
        } catch {
            compatTableProblem = error.reason
        }
    }

    private func startTapWhenPermitted() {
        guard let core, tap == nil else { return }
        let engine = core.engine()
        let sink = SystemEventSink(shortcuts: keyboardLayout.shortcuts)
        let injector = Injector(sink: sink, pasteboard: pasteboard)
        let controller = ExpansionController(
            engine: engine, injector: injector, translator: keyboardLayout.translator
        )
        // Rust calls this from the tap thread, so it does nothing but hand the
        // session to the main actor, where the panel lives.
        controller.setSessionHandler { [weak self] session in
            Task { @MainActor in self?.sessionStarted(session) }
        }
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
        self.controller = controller
        palette = PaletteStore(
            core: core,
            inserter: PaletteInserter(controller: controller) { [weak self] in self?.paletteTarget }
        )

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
        controller = nil
        palette = nil
        paletteTarget = nil
        // The session cannot be finished without an injector, so it goes too.
        form?.cancel()
        form = nil
        keyboardOwners = []
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

/// A window of Aralo's own that can hold the keyboard.
private enum KeyboardOwner {
    case palette, form, testField
}

/// The core's end of `onLibraryChange`. Rust calls this on the thread that
/// watches the folder, so it does nothing but hand the event to the main actor
/// and return: the watch reports nothing else until it does.
private final class LibraryWatcher: CoreEvents, @unchecked Sendable {
    private let handler: @Sendable (LibraryEvent) -> Void

    init(handler: @escaping @Sendable (LibraryEvent) -> Void) {
        self.handler = handler
    }

    func libraryChanged(event: LibraryEvent) {
        handler(event)
    }
}
