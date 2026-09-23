import AraloBridge
import Foundation
import Observation

/// The search palette's model: what the list shows for what has been typed,
/// which row Enter would insert, and what happened to the last insertion.
///
/// It is the whole palette apart from the window: a second shell draws a field
/// and a list over this and decides nothing about ranking, about what an empty
/// query shows, or about which app the text goes into.
///
/// The palette is a picker, not an editor. It writes nothing to the library;
/// the only thing it changes anywhere is the text in the app the user came
/// from.
@MainActor
@Observable
public final class PaletteStore {
    /// What the palette shows for a query. The list is never longer than
    /// [`PaletteStore.limit`]: a picker is read at a glance, and the query is
    /// how a user reaches the rest.
    public private(set) var rows: [SnippetRow] = []

    /// The row Enter inserts. Kept inside `rows`, and nil only when nothing
    /// matched.
    public private(set) var selection: String?

    /// What the selected snippet expands to, for the pane beside the list.
    /// A placeholder previews as its own source until the evaluator lands.
    public private(set) var preview: String = ""

    /// What the search box holds.
    public var query: String = "" {
        didSet {
            guard query != oldValue else { return }
            reload()
        }
    }

    /// Why the last pick inserted nothing, in the palette's words. Nil once
    /// another pick works, and nil while nothing has been picked.
    ///
    /// It is the one place a user hears about a refusal: an expansion they
    /// typed into a password manager simply does not happen, but a snippet they
    /// picked from a list and watched do nothing needs an answer.
    public private(set) var refusal: String?

    /// At most this many rows, whatever the query.
    public static let limit: UInt32 = 50

    @ObservationIgnored private let core: Core
    @ObservationIgnored private let inserter: SnippetInserter

    public init(core: Core, inserter: SnippetInserter) {
        self.core = core
        self.inserter = inserter
        reload()
    }

    /// Reads the library again, for a palette that is being opened or that was
    /// open when a file changed underneath it.
    public func refresh() {
        reload()
    }

    /// What the palette opens with: the query cleared and the list back to the
    /// snippets used most recently.
    public func reset() {
        refusal = nil
        query = ""
        // `didSet` did nothing if the query was already empty, and an opening
        // palette still needs the list read again.
        reload()
    }

    // MARK: - Moving through the list

    public func selectNext() {
        move(by: 1)
    }

    public func selectPrevious() {
        move(by: -1)
    }

    /// Selects a row the user clicked. An ID that is not in the list is
    /// ignored, so a click landing after a reload cannot select nothing.
    public func select(_ id: String) {
        guard rows.contains(where: { $0.id == id }) else { return }
        selection = id
        readPreview()
    }

    // MARK: - Inserting

    /// Inserts the selected snippet into the app the palette was opened over.
    ///
    /// The window must be out of the way before this is called: what is typed
    /// goes wherever the keyboard is. Returns true when the text is on its way
    /// in, which is when the palette is done. False means it should come back
    /// with `refusal` showing, because a picker that vanishes without inserting
    /// anything has said nothing.
    @discardableResult
    public func insertSelected() async -> Bool {
        guard let selection else { return false }
        refusal = nil
        guard let failure = await inserter.insert(snippetId: selection) else { return true }
        refusal = Self.describe(failure)
        return false
    }

    /// What a failure says to the user. The reasons are the core's; these are
    /// the words for them, and they name what to do about it where there is
    /// something to do.
    static func describe(_ failure: InsertFailure) -> String {
        switch failure {
        case .refused(.paused):
            "Aralo is paused. Resume it from the menu bar, and pick the snippet again."
        case .refused(.excludedApp):
            "Aralo does not type into this app: everything in it is treated as a password."
        case .refused(.snippetGone):
            "That snippet is not in the library any more."
        case .noTargetApp:
            "Aralo could not get back to the app you came from, so it typed nothing."
        }
    }

    private func move(by offset: Int) {
        guard !rows.isEmpty else { return }
        let current = rows.firstIndex { $0.id == selection } ?? 0
        // Clamped rather than wrapped: Down at the bottom of a list stays at
        // the bottom, which is what every other list on the Mac does.
        let next = min(max(current + offset, 0), rows.count - 1)
        selection = rows[next].id
        readPreview()
    }

    /// The list for the query, and the selection kept on the same snippet when
    /// it is still there. A new query starts at the top, which is the row Enter
    /// is most likely meant for.
    private func reload() {
        rows = query.isEmpty ? recentlyUsed() : matches()
        if let selection, rows.contains(where: { $0.id == selection }) {
            readPreview()
        } else {
            selection = rows.first?.id
            readPreview()
        }
    }

    private func matches() -> [SnippetRow] {
        core.search(
            query: SearchQuery(
                text: query,
                group: nil,
                tag: nil,
                // A switched-off snippet does not expand when it is typed, so
                // the palette does not offer it either.
                enabledOnly: true,
                limit: Self.limit
            )
        ).map(SnippetRow.init)
    }

    /// What an empty query shows: the snippets expanded most recently, then the
    /// rest of the library in its own order.
    ///
    /// Recents come from the index. Without one there is nothing to rank by,
    /// and the list order is what is left, which is a library a user can still
    /// pick from.
    private func recentlyUsed() -> [SnippetRow] {
        let all = core.search(
            query: SearchQuery(text: "", group: nil, tag: nil, enabledOnly: true, limit: Self.limit)
        ).map(SnippetRow.init)
        let byID = Dictionary(all.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        let recents = core.recents(limit: Self.limit).compactMap { byID[$0] }
        let used = Set(recents.map(\.id))
        return recents + all.filter { !used.contains($0.id) }
    }

    private func readPreview() {
        preview = selection.flatMap { core.preview(id: $0) } ?? ""
    }
}

/// What the palette does with the snippet the user picked: hand it to the
/// engine and run the plan in the app the palette was opened over.
///
/// A protocol because the model is the same wherever the text ends up: the app
/// wires in the real injector, and a test wires in something that writes down
/// what it was asked for.
@MainActor
public protocol SnippetInserter {
    /// Inserts `snippetId` into the app the palette came from. Nil when the
    /// text is on its way in; otherwise why nothing is.
    func insert(snippetId: String) async -> InsertFailure?
}

/// Why a pick inserted nothing: what the core refused, and the one thing only
/// the shell can get wrong.
public enum InsertFailure: Equatable, Sendable {
    /// The core inserts nothing, and why. The same pick works later, or with
    /// another app in front.
    case refused(InsertRefusal)
    /// The app the palette was opened over could not be brought back, so there
    /// is nowhere for the text to go. Nothing was inserted anywhere: the plan
    /// is never run against whatever else happens to have the keyboard.
    case noTargetApp
}
