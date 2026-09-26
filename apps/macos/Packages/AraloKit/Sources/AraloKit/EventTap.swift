import CoreGraphics
import Foundation

/// An active session-level event tap on its own thread.
///
/// Active, not listen-only: undo and delimiter handling must be able to
/// swallow a key. macOS disables a tap whose callback stalls, so the handler
/// must return in well under a millisecond, and the tap re-enables itself if
/// the system switches it off anyway.
public final class EventTap: @unchecked Sendable {
    public enum StartError: Error {
        /// Accessibility or Input Monitoring has not been granted.
        case notPermitted
    }

    public typealias Handler = @Sendable (TapEvent) -> Bool

    /// What happened to the tap itself, for the local counters.
    public enum Health: Sendable {
        /// The system switched the tap off because a callback took too long.
        case timedOut
        /// The system switched the tap off for user input.
        case disabledByUserInput
        /// The tap was switched back on.
        case reenabled
    }

    public typealias HealthHandler = @Sendable (Health) -> Void

    private let handler: Handler
    /// Called on the tap thread, so it must be as quick as `handler`.
    private let onHealth: HealthHandler?
    private let lock = NSLock()
    private var port: CFMachPort?
    private var runLoop: CFRunLoop?
    private var reenables = 0

    public init(handler: @escaping Handler, onHealth: HealthHandler? = nil) {
        self.handler = handler
        self.onHealth = onHealth
    }

    /// How often the system disabled the tap and it was switched back on. A
    /// local counter for diagnostics; it never leaves the machine.
    public var reenableCount: Int {
        lock.withLock { reenables }
    }

    public var isRunning: Bool {
        lock.withLock { port != nil }
    }

    public func start() throws {
        guard !isRunning else { return }
        let types: [CGEventType] = [.keyDown, .leftMouseDown, .rightMouseDown, .otherMouseDown]
        let mask = types.reduce(CGEventMask(0)) { $0 | (CGEventMask(1) << $1.rawValue) }
        guard let port = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .headInsertEventTap,
            options: .defaultTap,
            eventsOfInterest: mask,
            callback: eventTapCallback,
            userInfo: Unmanaged.passUnretained(self).toOpaque()
        ) else {
            throw StartError.notPermitted
        }
        lock.withLock { self.port = port }

        let thread = Thread { [self] in
            guard let port = lock.withLock({ self.port }) else { return }
            let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, port, 0)
            let current = CFRunLoopGetCurrent()
            CFRunLoopAddSource(current, source, .commonModes)
            // The watchdog: the disabled-by-timeout event is not guaranteed to arrive.
            let watchdog = CFRunLoopTimerCreateWithHandler(
                kCFAllocatorDefault, CFAbsoluteTimeGetCurrent() + 5, 5, 0, 0
            ) { [weak self] _ in
                self?.reenableIfNeeded()
            }
            CFRunLoopAddTimer(current, watchdog, .commonModes)
            lock.withLock { runLoop = current }
            CGEvent.tapEnable(tap: port, enable: true)
            CFRunLoopRun()
        }
        thread.name = "app.aralo.tap"
        thread.qualityOfService = .userInteractive
        thread.start()
    }

    public func stop() {
        let (port, runLoop) = lock.withLock { () -> (CFMachPort?, CFRunLoop?) in
            defer {
                self.port = nil
                self.runLoop = nil
            }
            return (self.port, self.runLoop)
        }
        if let port {
            CGEvent.tapEnable(tap: port, enable: false)
            CFMachPortInvalidate(port)
        }
        if let runLoop {
            CFRunLoopStop(runLoop)
        }
    }

    fileprivate func reenableIfNeeded() {
        guard let port = lock.withLock({ port }), !CGEvent.tapIsEnabled(tap: port) else { return }
        CGEvent.tapEnable(tap: port, enable: true)
        lock.withLock { reenables += 1 }
        onHealth?(.reenabled)
    }

    fileprivate func receive(type: CGEventType, event: CGEvent) -> Bool {
        switch type {
        case .tapDisabledByTimeout, .tapDisabledByUserInput:
            onHealth?(type == .tapDisabledByTimeout ? .timedOut : .disabledByUserInput)
            reenableIfNeeded()
            return false
        case .keyDown:
            // Our own output: let it through and keep it away from the engine.
            if event.getIntegerValueField(.eventSourceUserData) == SystemEventSink.selfEventTag {
                return false
            }
            return handler(.key(Self.stroke(from: event)))
        case .leftMouseDown, .rightMouseDown, .otherMouseDown:
            return handler(.mouseDown)
        default:
            return false
        }
    }

    private static func stroke(from event: CGEvent) -> KeyStroke {
        var length = 0
        var units = [UniChar](repeating: 0, count: 8)
        event.keyboardGetUnicodeString(maxStringLength: units.count, actualStringLength: &length, unicodeString: &units)
        let text = String(utf16CodeUnits: units, count: min(length, units.count))
        // The stack copy held a keystroke; do not leave it behind.
        for index in units.indices { units[index] = 0 }
        return KeyStroke(
            keyCode: CGKeyCode(event.getIntegerValueField(.keyboardEventKeycode)),
            flags: event.flags,
            text: text
        )
    }
}

private func eventTapCallback(
    proxy: CGEventTapProxy,
    type: CGEventType,
    event: CGEvent,
    userInfo: UnsafeMutableRawPointer?
) -> Unmanaged<CGEvent>? {
    guard let userInfo else { return Unmanaged.passUnretained(event) }
    let tap = Unmanaged<EventTap>.fromOpaque(userInfo).takeUnretainedValue()
    return tap.receive(type: type, event: event) ? nil : Unmanaged.passUnretained(event)
}
