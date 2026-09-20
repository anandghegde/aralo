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

/// Keeps the shortcut key codes in step with the keyboard layout, so Cmd+V is
/// still paste after the user switches from US to Dvorak or AZERTY.
@MainActor
public final class KeyboardLayoutMonitor {
    public let shortcuts = ShortcutKeyCodes()
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
    }
}
