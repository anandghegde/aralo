import AraloBridge
@testable import AraloHarness
import Foundation

/// A text field, a clipboard and a pretend Aralo, all in memory. Time moves
/// only when the runner waits.
@MainActor
final class FakeDesktop: Desktop {
    /// What the pretend Aralo knows: what is typed, and what replaces it.
    var snippets: [String: String] = [:]
    var expansionDelay: TimeInterval = 0.02
    /// Expansions that do nothing, counted down: the first N are swallowed.
    var deafExpansions = 0
    var speaksAccessibility = true
    var restoresClipboard = true
    var undoWorks = true
    var front: String? = "com.example.Editor"
    /// Another app comes forward once this many keys have been sent.
    var stealsFocusAfterKeys: Int?
    var bringUpResult = BringUp.ready
    var canType = true

    var field = ""
    private(set) var keysSent = 0
    private(set) var dismissed: [String] = []
    private var clock: TimeInterval = 100
    private var allSelected = false
    private var due: [(at: TimeInterval, run: () -> Void)] = []
    private var lastExpansion: (typed: String, text: String)?
    var clipboard: String?

    func bringUp(_ app: CompatApp, recipe: Recipe) -> BringUp { bringUpResult }
    func dismiss(_ app: CompatApp) { dismissed.append(app.bundleId) }
    func frontmostBundleID() -> String? { front }
    func focusedText() -> String? { speaksAccessibility ? field : nil }
    func now() -> TimeInterval { clock }

    func wait(_ seconds: TimeInterval) {
        clock += seconds
        let ready = due.filter { $0.at <= clock }
        due.removeAll { $0.at <= clock }
        ready.forEach { $0.run() }
    }

    func press(_ chord: KeyChord) {
        countKey()
        switch chord {
        case .selectAll: allSelected = true
        case .delete where allSelected: field = ""
        case .killLine: field = ""
        case .copy where allSelected: clipboard = field
        case .undo: undo()
        default: break
        }
        if chord != .selectAll { allSelected = false }
    }

    func type(_ text: String) -> Bool {
        guard canType else { return false }
        for character in text {
            countKey()
            if allSelected { field = "" }
            allSelected = false
            field.append(character)
            expandIfTyped()
        }
        return true
    }

    private func countKey() {
        keysSent += 1
        if let limit = stealsFocusAfterKeys, keysSent >= limit { front = "com.example.Intruder" }
    }

    private func expandIfTyped() {
        guard let (typed, text) = snippets.first(where: { field.hasSuffix($0.key) }) else { return }
        if deafExpansions > 0 {
            deafExpansions -= 1
            return
        }
        due.append((clock + expansionDelay, { [self] in
            guard field.hasSuffix(typed) else { return }
            field.removeLast(typed.count)
            field += text
            lastExpansion = (typed, text)
            if !restoresClipboard { clipboard = text }
        }))
    }

    private func undo() {
        guard undoWorks, let last = lastExpansion, field.hasSuffix(last.text) else { return }
        field.removeLast(last.text.count)
        field += last.typed
        lastExpansion = nil
    }
}

func app(_ bundleID: String = "com.example.Editor", name: String = "Editor") -> CompatApp {
    CompatApp(
        name: name, bundleId: bundleID,
        profile: InjectionProfile(
            insert: .auto, typingLimit: 120, keyDelayMs: 1, pasteSettleMs: 250, undo: .native, delete: .backspace
        )
    )
}
