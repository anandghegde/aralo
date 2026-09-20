import AppKit
import Carbon.HIToolbox

/// Polls secure input. While a password field holds it, key events stop
/// arriving at the tap; the app shows that instead of looking broken.
@MainActor
public final class SecureInputMonitor {
    private var timer: Timer?
    private var last = false
    private let onChange: @MainActor (Bool) -> Void

    public init(onChange: @escaping @MainActor (Bool) -> Void) {
        self.onChange = onChange
    }

    public var isSecureInputOn: Bool { IsSecureEventInputEnabled() }

    /// The app that turned secure input on, when the system says. An app can
    /// leave it on by mistake, long after its password field is gone, and the
    /// user can only fix what has a name.
    public var holder: SecureInputHolder? {
        guard isSecureInputOn,
              let session = CGSessionCopyCurrentDictionary() as? [String: Any],
              let pid = Self.holderPID(in: session)
        else { return nil }
        let app = NSRunningApplication(processIdentifier: pid)
        return SecureInputHolder(pid: pid, name: app?.localizedName, bundleID: app?.bundleIdentifier)
    }

    /// The window server's session dictionary carries the process under this
    /// key while secure input is on. The key is not in a public header, so its
    /// absence is an ordinary answer: the menu then names nobody.
    nonisolated static func holderPID(in session: [String: Any]) -> pid_t? {
        guard let pid = (session["kCGSSessionSecureInputPID"] as? NSNumber)?.int32Value, pid > 0 else {
            return nil
        }
        return pid
    }

    public func start(interval: TimeInterval = 1) {
        stop()
        check()
        timer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.check() }
        }
    }

    public func stop() {
        timer?.invalidate()
        timer = nil
    }

    private func check() {
        let now = IsSecureEventInputEnabled()
        if now != last {
            last = now
            onChange(now)
        }
    }
}

public struct SecureInputHolder: Equatable, Sendable {
    public let pid: pid_t
    /// Absent for a process that is not an app, a login window for example.
    public let name: String?
    public let bundleID: String?
}

/// Tells the engine which app receives keys. A change of app resets the buffer.
@MainActor
public final class FrontAppMonitor {
    private var observer: NSObjectProtocol?
    private let onChange: @MainActor (String) -> Void

    public init(onChange: @escaping @MainActor (String) -> Void) {
        self.onChange = onChange
    }

    public func start() {
        stop()
        report(NSWorkspace.shared.frontmostApplication)
        observer = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main
        ) { [weak self] notification in
            let app = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication
            let bundleID = app?.bundleIdentifier ?? ""
            MainActor.assumeIsolated { self?.onChange(bundleID) }
        }
    }

    public func stop() {
        if let observer {
            NSWorkspace.shared.notificationCenter.removeObserver(observer)
        }
        observer = nil
    }

    private func report(_ app: NSRunningApplication?) {
        onChange(app?.bundleIdentifier ?? "")
    }
}

/// Keeps the shortcut key codes and the key translator in step with the input
/// source, so Cmd+V is still paste after the user switches from US to Dvorak
/// or AZERTY, dead keys follow the new layout, and matching pauses while an
/// input method composes.
@MainActor
public final class KeyboardLayoutMonitor {
    public let shortcuts = ShortcutKeyCodes()
    public let translator = KeyTranslator()
    /// Called after every change, and once from `start`.
    public var onChange: (@MainActor () -> Void)?
    private var observer: NSObjectProtocol?

    public init() {}

    public func start() {
        stop()
        refresh()
        let layoutChanged = Notification.Name(kTISNotifySelectedKeyboardInputSourceChanged as String)
        observer = DistributedNotificationCenter.default().addObserver(
            forName: layoutChanged, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
    }

    public func stop() {
        if let observer {
            DistributedNotificationCenter.default().removeObserver(observer)
        }
        observer = nil
    }

    private func refresh() {
        shortcuts.replace(with: KeyboardLayout.currentKeyCodes())
        translator.replace(
            layout: KeyboardLayout.currentLayout(), composing: KeyboardLayout.currentInputSourceComposes()
        )
        onChange?()
    }
}
