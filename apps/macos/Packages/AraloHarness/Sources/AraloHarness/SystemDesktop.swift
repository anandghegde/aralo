import AppKit
import ApplicationServices
import AraloBridge
import AraloKit
import Carbon.HIToolbox

/// The real desktop: real apps, real key events, the real clipboard. The
/// process that runs it needs Accessibility access.
@MainActor
public final class SystemDesktop: Desktop {
    /// How long a person gets to click into a field, for a `manual` recipe.
    /// Zero means nobody is there, and those apps are skipped.
    public var manualWait: TimeInterval = 0
    /// Where instructions for that person go.
    public var say: (String) -> Void = { _ in }

    private let scratch: URL
    private let source = CGEventSource(stateID: .combinedSessionState)
    private var plainKeys: [Character: CGKeyCode] = [:]
    private var launched: Set<String> = []

    public init() throws {
        scratch = FileManager.default.temporaryDirectory
            .appendingPathComponent("aralo-matrix-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: scratch, withIntermediateDirectories: true)
        plainKeys = Self.plainKeyCodes()
    }

    /// Removes the scratch files. Call when the run is over.
    public func cleanUp() {
        try? FileManager.default.removeItem(at: scratch)
    }

    // MARK: Apps

    public func bringUp(_ app: CompatApp, recipe: Recipe) -> BringUp {
        guard let appURL = NSWorkspace.shared.urlForApplication(withBundleIdentifier: app.bundleId) else {
            return .notInstalled
        }
        if recipe.open == .manual, manualWait <= 0 {
            return .unreachable("needs a person to click into a field")
        }
        let wasRunning = !NSRunningApplication.runningApplications(withBundleIdentifier: app.bundleId).isEmpty
        let documents: [URL]
        do {
            documents = try scratchFiles(for: recipe)
        } catch {
            return .unreachable("could not write a scratch file")
        }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        if documents.isEmpty {
            NSWorkspace.shared.openApplication(at: appURL, configuration: configuration)
        } else {
            NSWorkspace.shared.open(documents, withApplicationAt: appURL, configuration: configuration)
        }
        if !wasRunning { launched.insert(app.bundleId) }

        guard waitUntilFront(app.bundleId, timeout: 20) else { return .unreachable("did not come to the front") }
        Accessibility.wakeTree(ofAppWithBundleID: app.bundleId)
        wait(recipe.settle)
        return takeFocus(in: app, recipe: recipe)
    }

    private func takeFocus(in app: CompatApp, recipe: Recipe) -> BringUp {
        for chord in recipe.chords {
            guard isFront(app.bundleId) else { return .unreachable("lost the keyboard while opening a field") }
            press(chord)
            wait(0.4)
        }
        if recipe.open == .manual {
            say("Click into a text field in \(app.name). Typing starts in \(Int(manualWait)) seconds.")
            wait(manualWait)
        }
        return isFront(app.bundleId) ? .ready : .unreachable("was not in front when typing was due")
    }

    public func dismiss(_ app: CompatApp) {
        guard launched.remove(app.bundleId) != nil else { return }
        let running = NSRunningApplication.runningApplications(withBundleIdentifier: app.bundleId)
        running.forEach { $0.terminate() }
        wait(2)
        // A document with changes, or a shell with a job, asks before it quits.
        running.filter { !$0.isTerminated }.forEach { $0.forceTerminate() }
    }

    private func scratchFiles(for recipe: Recipe) throws -> [URL] {
        let name = "aralo-matrix-" + recipe.bundleID.replacingOccurrences(of: ".", with: "-")
        switch recipe.open {
        case .document:
            let file = scratch.appendingPathComponent(name).appendingPathExtension(recipe.fileExtension ?? "txt")
            try Data().write(to: file)
            return [file]
        case .webPage:
            let file = scratch.appendingPathComponent(name).appendingPathExtension("html")
            try Data(Self.page.utf8).write(to: file)
            return [file]
        case .shell:
            let file = scratch.appendingPathComponent(name).appendingPathExtension("command")
            try Data(Self.script.utf8).write(to: file)
            try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: file.path)
            return [file]
        case .keys, .manual:
            return []
        }
    }

    private static let page = """
        <!doctype html>
        <meta charset="utf-8">
        <title>Aralo injection matrix</title>
        <textarea autofocus rows="20" cols="100"></textarea>
        <script>document.querySelector("textarea").focus()</script>

        """

    /// A line editor that runs nothing: Return starts another empty line.
    /// zsh's editor rather than `cat`, because the terminal driver's own line
    /// buffer stops at 1,024 bytes and the long case is 2,000 characters.
    private static let script = """
        #!/bin/zsh -f
        clear
        while true; do
          line=""
          vared -p "aralo-matrix> " line || exit 0
        done

        """

    // MARK: The keyboard

    public func frontmostBundleID() -> String? {
        Accessibility.frontmostPID().flatMap { NSRunningApplication(processIdentifier: $0)?.bundleIdentifier }
            ?? NSWorkspace.shared.frontmostApplication?.bundleIdentifier
    }

    public func focusedText() -> String? {
        Accessibility.frontmostPID().flatMap(Accessibility.focusedText(ofAppWithPID:))
    }

    public func press(_ chord: KeyChord) {
        guard let keyCode = keyCode(for: chord) else { return }
        var flags: CGEventFlags = []
        if chord.modifiers.contains(.cmd) { flags.insert(.maskCommand) }
        if chord.modifiers.contains(.ctrl) { flags.insert(.maskControl) }
        if chord.modifiers.contains(.opt) { flags.insert(.maskAlternate) }
        if chord.modifiers.contains(.shift) { flags.insert(.maskShift) }
        post(keyCode, flags)
    }

    public func type(_ text: String) -> Bool {
        let keyCodes = text.map { $0 == " " ? CGKeyCode(kVK_Space) : plainKeys[$0] }
        // All or nothing: half an abbreviation is worse than none.
        guard keyCodes.allSatisfy({ $0 != nil }) else { return false }
        keyCodes.compactMap { $0 }.forEach { post($0, []) }
        return true
    }

    /// Untagged events, so Aralo's tap reads them as it reads a keyboard.
    private func post(_ keyCode: CGKeyCode, _ flags: CGEventFlags) {
        for keyDown in [true, false] {
            guard let event = CGEvent(keyboardEventSource: source, virtualKey: keyCode, keyDown: keyDown) else {
                continue
            }
            event.flags = flags
            event.post(tap: .cghidEventTap)
            wait(0.006)
        }
    }

    private func keyCode(for chord: KeyChord) -> CGKeyCode? {
        switch chord.key {
        case .space: return CGKeyCode(kVK_Space)
        case .tab: return CGKeyCode(kVK_Tab)
        case .return: return CGKeyCode(kVK_Return)
        case .delete: return CGKeyCode(kVK_Delete)
        case .escape: return CGKeyCode(kVK_Escape)
        case .character(let character):
            // With Command held, apps go by what the key gives under Command.
            let underCommand = chord.modifiers.contains(.cmd)
                ? KeyboardLayout.currentKeyCodes(for: [character])[character] : nil
            return underCommand ?? plainKeys[character]
        }
    }

    /// The unshifted key for each character, in the keyboard layout in use.
    private static func plainKeyCodes() -> [Character: CGKeyCode] {
        let translator = KeyTranslator(layout: KeyboardLayout.currentLayout())
        var found: [Character: CGKeyCode] = [:]
        for keyCode in CGKeyCode(0)..<128 {
            let text = translator.text(keyCode: keyCode, flags: [])
            translator.clear()
            guard let text, text.count == 1, let character = text.first, found[character] == nil else { continue }
            found[character] = keyCode
        }
        return found
    }

    // MARK: Clipboard and time

    public var clipboard: String? {
        get { NSPasteboard.general.string(forType: .string) }
        set {
            NSPasteboard.general.clearContents()
            if let newValue { NSPasteboard.general.setString(newValue, forType: .string) }
        }
    }

    /// Runs the run loop rather than sleeping, so AppKit keeps up with which
    /// apps are running.
    public func wait(_ seconds: TimeInterval) {
        RunLoop.current.run(until: Date(timeIntervalSinceNow: seconds))
    }

    public func now() -> TimeInterval {
        ProcessInfo.processInfo.systemUptime
    }

    private func isFront(_ bundleID: String) -> Bool {
        frontmostBundleID()?.caseInsensitiveCompare(bundleID) == .orderedSame
    }

    private func waitUntilFront(_ bundleID: String, timeout: TimeInterval) -> Bool {
        let deadline = now() + timeout
        while now() < deadline {
            if isFront(bundleID) { return true }
            wait(0.1)
        }
        return false
    }
}
