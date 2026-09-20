import AraloBridge
import Foundation

/// Everything the runner asks of the operating system. `SystemDesktop` is the
/// real one; the tests drive the runner against a field that lives in memory.
@MainActor
public protocol Desktop: AnyObject {
    /// Gets a text field in `app` ready for keys.
    func bringUp(_ app: CompatApp, recipe: Recipe) -> BringUp
    /// Quits the app if `bringUp` was what started it.
    func dismiss(_ app: CompatApp)
    /// Bundle ID of the app that has the keyboard now.
    func frontmostBundleID() -> String?
    /// The focused element's text, through the Accessibility API. Nil when the
    /// app does not say.
    func focusedText() -> String?
    /// Presses one chord with real key events, the way a keyboard would.
    func press(_ chord: KeyChord)
    /// Types lower-case letters and spaces, one real key event each. False
    /// when the keyboard layout has no key for one of them.
    func type(_ text: String) -> Bool
    var clipboard: String? { get set }
    func wait(_ seconds: TimeInterval)
    /// Seconds on a clock that never goes back.
    func now() -> TimeInterval
}

public enum BringUp: Equatable, Sendable {
    case ready
    case notInstalled
    /// The app never came to the front, or nobody clicked into a field.
    case unreachable(String)
}
