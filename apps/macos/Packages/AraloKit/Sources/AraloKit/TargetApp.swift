import AppKit
import AraloBridge
import Foundation

/// The app a palette insertion is for: the one that had the keyboard when the
/// hot key was pressed, and how to make sure it has it again before anything
/// is typed.
///
/// A protocol because everything above it is the same whatever brings the app
/// back; the tests wire in an app that comes forward, one that does not, and
/// one that was never there.
@MainActor
public protocol TargetApp {
    /// The bundle ID the text is for. The core is told this, so the insertion
    /// follows that app's row of the compatibility table.
    var bundleId: String { get }
    /// Brings the app forward, and returns once it has the keyboard. False
    /// when it never came forward, in which case nothing may be typed: the
    /// keys would land in whatever else is in front.
    func activate() async -> Bool
}

/// The app macOS says was in front. Activating it is usually nothing at all:
/// the palette is a panel that does not take the keyboard away from it, so the
/// app is still in front and this returns at once.
@MainActor
public struct RunningTargetApp: TargetApp {
    public let bundleId: String
    private let app: NSRunningApplication

    /// The process, for asking Accessibility what is selected in it.
    public var processIdentifier: pid_t { app.processIdentifier }

    /// What the app calls itself, such as "Mail": what a snippet whose AI
    /// block declared `app` sends.
    public var name: String? { app.localizedName }

    /// How long the app gets to come forward after a click has made Aralo the
    /// active one. Longer than a switch takes, short enough that a user who is
    /// waiting for text sees no lag before the message that none is coming.
    public static let patience = Duration.milliseconds(600)
    private static let step = Duration.milliseconds(10)

    /// Nil for an app with no bundle ID, which is what a process that is not
    /// an app in the usual sense has. There is no row of the compatibility
    /// table for it and no way to name it to the core.
    public init?(_ app: NSRunningApplication) {
        guard let bundleId = app.bundleIdentifier else { return nil }
        self.app = app
        self.bundleId = bundleId
    }

    /// The app in front now, for a palette that is being opened.
    public static func frontmost() -> RunningTargetApp? {
        NSWorkspace.shared.frontmostApplication.flatMap { RunningTargetApp($0) }
    }

    public func activate() async -> Bool {
        if isReady { return true }
        app.activate()
        // `activate` returns before the app has the keyboard, and macOS says
        // nothing when it does, so watch for it.
        let deadline = ContinuousClock.now + Self.patience
        while ContinuousClock.now < deadline {
            try? await Task.sleep(for: Self.step)
            if isReady { return true }
        }
        return false
    }

    /// The app is in front and running. An app that quit while the palette was
    /// open is not somewhere to type.
    private var isReady: Bool {
        !app.isTerminated && app.isActive
    }
}

/// Inserts a snippet the user picked into the app the palette was opened over:
/// brings that app back, asks the engine for the plan, and runs it on the queue
/// every other expansion runs on.
///
/// The app is brought back first and the plan asked for second, in that order:
/// the core takes the app it is given as the one that has the keyboard, and an
/// expansion must never be planned for an app the text is not going into.
///
/// The palette's window must have given the keyboard back before this is
/// called. Synthetic keys follow the keyboard, so a snippet inserted while the
/// palette is still up lands in the search box.
@MainActor
public final class PaletteInserter: SnippetInserter {
    private let controller: ExpansionController
    private let target: @MainActor () -> TargetApp?

    public init(controller: ExpansionController, target: @escaping @MainActor () -> TargetApp?) {
        self.controller = controller
        self.target = target
    }

    public func insert(snippetId: String) async -> InsertFailure? {
        guard let target = target(), await target.activate() else { return .noTargetApp }
        guard let refused = controller.insert(snippetId: snippetId, into: target.bundleId) else {
            return nil
        }
        return .refused(refused)
    }
}
