import AraloBridge
import Foundation

// The panels that open over another app: the search palette, the command
// panel, and what the form panel needs for a snippet's AI blocks. They note
// the app in front, take the keyboard while they are up, and give it back
// before anything is typed into that app.
extension AraloService {
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

    // MARK: - Commands on selected text

    /// The AI settings in Aralo's state folder, with keys in the keychain.
    /// One instance for the settings pane and the command panel, so the two
    /// never disagree about a profile.
    public func aiSettings() throws -> AiProfiles {
        if let aiProfiles { return aiProfiles }
        let path = cacheURL.appendingPathComponent("profiles.toml").path
        let opened = try AiProfiles.open(path: path, keys: .keychain)
        aiProfiles = opened
        return opened
    }

    /// Reads what is selected in the app in front, then asks the shell for the
    /// command panel.
    ///
    /// The selection is read first, while that app still has the keyboard: a
    /// copy made with the panel up would copy from the panel. A selection that
    /// cannot be read still opens the panel, to say why.
    public func openCommands() {
        guard let core, let controller, !isReadingSelection,
              let target = RunningTargetApp.frontmost() else { return }
        let runner: CommandRunner
        do {
            runner = try aiSettings()
        } catch {
            runner = UnavailableRunner(reason: error.reason)
        }
        let capture = SelectionCapture(controller: controller, target: target) { [weak self] in
            self?.secureInput?.isSecureInputOn == true
        }
        isReadingSelection = true
        Task { @MainActor in
            let selection = await capture.read()
            isReadingSelection = false
            if case .failure(let failure) = selection, failure != .nothingSelected {
                core.recordShellEvent(event: .selectionUnreadable)
            }
            // The tap may have gone down while the app was copying.
            guard self.controller != nil else { return }
            commands?.close()
            commands = CommandStore(
                commands: core.commands(), selection: selection, runner: runner, access: capture
            )
            onCommandsRequested?()
        }
    }

    /// Whether the command panel has the keyboard, on the palette's terms.
    /// The panel gives it back before it replaces anything.
    public func setCommandsHaveKeyboard(_ hasKeyboard: Bool) {
        setKeyboard(.command, hasKeyboard)
    }

    /// The panel has gone. Whatever was running stops, and the selection it
    /// held is forgotten.
    public func commandsClosed() {
        commands?.close()
        commands = nil
        setKeyboard(.command, false)
    }

    // MARK: - AI blocks in snippets

    /// What a session may read from the app the text is for: the clipboard
    /// for its body, and for its AI blocks whatever their snippet declared.
    /// Each is read only when the session asks for it, and the selection not
    /// at all while secure input is on or from a password field.
    func sessionContext(for target: RunningTargetApp?) -> SessionContext {
        SessionContext(
            clipboard: { [pasteboard] in pasteboard.text() },
            selection: { [weak self] in
                guard let target, self?.secureInput?.isSecureInputOn != true,
                      case .text(let text) = AccessibilitySelection.read(pid: target.processIdentifier)
                else { return nil }
                return text
            },
            app: { target?.name },
            window: { target.flatMap { AccessibilitySelection.windowTitle(pid: $0.processIdentifier) } }
        )
    }

    /// Answers AI blocks and the editor's actions with the AI settings,
    /// opened the first time either asks, unless search by meaning opened
    /// them at start to follow the switch. The keychain is read only when a
    /// model is asked something.
    var aiRunner: DeferredAIRunner {
        DeferredAIRunner { [weak self] in
            guard let self else { throw AiBridgeError.Failed(message: "Aralo is closing.") }
            return try aiSettings()
        }
    }
}

/// Stands in for the AI settings when they could not be opened, so the panel
/// still opens and says why nothing runs.
private struct UnavailableRunner: CommandRunner {
    let reason: String

    func start(_ command: AiCommand, selection: String) async throws -> AiCommandRunProtocol {
        throw Unavailable(errorDescription: reason)
    }

    private struct Unavailable: LocalizedError {
        var errorDescription: String?
    }
}
