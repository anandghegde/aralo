import AraloBridge
@testable import AraloHarness
import Foundation

/// A text field, a clipboard and a pretend Aralo, all in memory. Time moves
/// only when the runner waits.
@MainActor
final class FakeDesktop: Desktop {
    /// What the pretend Aralo knows: what is typed, and what replaces it.
    /// `{caret}` in a body is where the caret is left, the way `{{cursor}}`
    /// leaves it.
    var snippets: [String: String] = [:]
    /// Bodies that ask something first: what is typed, and what replaces it once
    /// the panel is answered, with `{answer}` where the answer goes.
    var forms: [String: String] = [:]
    /// False for an Aralo whose panel never opens, so the answer is typed into
    /// the document instead.
    var formPanelOpens = true
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
    /// Where the next character goes, in characters from the start.
    private var caret = 0
    /// The form panel, while it is up.
    private struct Panel {
        /// What the expansion will replace. The delimiter was swallowed to open
        /// the panel, so only the abbreviation is left in the document.
        var abbreviation: String
        /// What it inserts once it is answered, with `{answer}` where the answer
        /// goes.
        var body: String
        /// What has been typed into it so far.
        var answer = ""
    }

    private var panel: Panel?
    var clipboard: String?

    static let answerMarker = "{answer}"
    static let caretMarker = "{caret}"

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
        case .delete where allSelected: empty()
        case .killLine: empty()
        case .copy where allSelected: clipboard = field
        case .undo: undo()
        case .return where panel != nil: submitForm()
        default: break
        }
        if chord != .selectAll { allSelected = false }
    }

    func type(_ text: String) -> Bool {
        guard canType else { return false }
        for character in text {
            countKey()
            // While the panel is up the keyboard is its own: nothing reaches
            // the document until the answer is in.
            if panel != nil {
                panel?.answer.append(character)
                continue
            }
            if allSelected { empty() }
            allSelected = false
            insert(String(character))
            if askIfTyped() { continue }
            expandIfTyped()
        }
        return true
    }

    private func countKey() {
        keysSent += 1
        if let limit = stealsFocusAfterKeys, keysSent >= limit { front = "com.example.Intruder" }
    }

    private func expandIfTyped() {
        guard let (typed, body) = snippets.first(where: { field.hasSuffix($0.key) }) else { return }
        schedule(body, replacing: typed, retyping: typed)
    }

    /// A body with a question swallows the key that completed the match and puts
    /// a panel up: the document holds the abbreviation and nothing else until
    /// the answer is in.
    private func askIfTyped() -> Bool {
        guard formPanelOpens, let (typed, body) = forms.first(where: { field.hasSuffix($0.key) }) else { return false }
        remove(Expectation.delimiter.count)
        panel = Panel(abbreviation: String(typed.dropLast(Expectation.delimiter.count)), body: body)
        return true
    }

    /// Enter in the panel: the answer goes into the body, and the text arrives
    /// after the delay any other expansion takes.
    private func submitForm() {
        guard let form = panel else { return }
        panel = nil
        schedule(
            form.body.replacingOccurrences(of: Self.answerMarker, with: form.answer),
            replacing: form.abbreviation,
            retyping: form.abbreviation + Expectation.delimiter
        )
    }

    /// The pretend Aralo's one move: after `expansionDelay`, what was typed is
    /// taken back and the body is inserted in its place.
    private func schedule(_ body: String, replacing typed: String, retyping: String) {
        if deafExpansions > 0 {
            deafExpansions -= 1
            return
        }
        due.append((clock + expansionDelay, { [self] in
            guard field.hasSuffix(typed) else { return }
            remove(typed.count)
            let parts = body.components(separatedBy: Self.caretMarker)
            insert(parts.joined())
            // A body with a caret marker leaves the caret inside what it typed,
            // which is what `{{cursor}}` does.
            caret -= parts.dropFirst().joined().count
            lastExpansion = (retyping, parts.joined())
            if !restoresClipboard { clipboard = parts.joined() }
        }))
    }

    private func undo() {
        guard undoWorks, let last = lastExpansion, field.hasSuffix(last.text) else { return }
        caret = field.count
        remove(last.text.count)
        insert(last.typed)
        lastExpansion = nil
    }

    private func insert(_ text: String) {
        field.insert(contentsOf: text, at: field.index(field.startIndex, offsetBy: min(caret, field.count)))
        caret += text.count
    }

    /// Takes `count` characters back from the caret, which is where a deletion
    /// happens: what was typed is always what is just before it.
    private func remove(_ count: Int) {
        let end = field.index(field.startIndex, offsetBy: min(caret, field.count))
        let start = field.index(end, offsetBy: -min(count, caret))
        field.removeSubrange(start..<end)
        caret = field.distance(from: field.startIndex, to: start)
    }

    private func empty() {
        field = ""
        caret = 0
        panel = nil
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
