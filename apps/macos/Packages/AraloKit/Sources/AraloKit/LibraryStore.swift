import AraloBridge
import Foundation
import Observation

/// The main window's model: the group tree, the snippet list and the draft on
/// screen. Every editing call the window makes goes through here, so the views
/// above it hold no logic a Windows shell would have to write again.
///
/// It is the core's view of the folder and nothing else. Nothing is cached
/// across an edit: each call writes a file and reads the folder back, so what
/// the list shows is what a text editor would show.
@MainActor
@Observable
public final class LibraryStore {
    /// The group tree, root first. Nil only before the first read.
    public private(set) var root: GroupNode?
    /// What the middle pane lists: the selected group, narrowed by the search
    /// box.
    public private(set) var rows: [SnippetRow] = []
    /// What the editor should say about the draft on screen. None of these
    /// stops a save.
    public private(set) var problems: [DraftProblem] = []
    /// The snippet being edited, or nil when the list has no selection.
    public var editing: Editing? {
        didSet {
            guard editing?.draft != oldValue?.draft || editing?.id != oldValue?.id else { return }
            problems = editing.map { core.checkDraft(draft: $0.draft, editing: $0.savedID) } ?? []
        }
    }

    /// The group the sidebar has selected. Its snippets, and those of every
    /// group inside it, are what the list shows.
    public private(set) var selectedGroup: [String] = []
    /// What the search box holds. It narrows the selected group rather than
    /// replacing it.
    public private(set) var query: String = ""

    /// Why the last read of the folder failed, when one did. The library that
    /// loaded stays in use, so this is something to show, not a stop.
    public private(set) var problem: String?

    /// Why the last editing call failed, in the core's words. The next call
    /// that works clears it.
    public private(set) var failure: String?

    @ObservationIgnored private let core: Core

    public init(core: Core) {
        self.core = core
        refresh()
    }

    /// Runs an editing call, keeping what went wrong instead of throwing it at
    /// a view that has nowhere to put it. Every shell needs this; the words
    /// are the core's, so none of them has to invent its own.
    public func attempt(_ work: (LibraryStore) throws -> Void) {
        do {
            try work(self)
            failure = nil
        } catch {
            failure = error.reason
        }
    }

    /// Forgets the last failure, for a window whose alert has been dismissed.
    public func dismissFailure() {
        failure = nil
    }

    // MARK: - Selection

    public func select(group path: [String]) {
        selectedGroup = path
        reloadRows()
    }

    public func search(_ text: String) {
        query = text
        reloadRows()
    }

    /// Opens a snippet in the editor. Passing nil clears it. Unsaved edits are
    /// dropped: the window asks first.
    public func select(snippet id: String?) {
        guard let id, let detail = core.snippet(id: id) else {
            editing = nil
            return
        }
        editing = Editing(detail)
    }

    /// The snippet the list has selected, for a list that binds to one ID.
    public var selectedSnippet: String? { editing?.id }

    // MARK: - Reading

    /// Reads the folder again: the tree, the list and, when it is not being
    /// edited, the open snippet. The window calls this on every library event.
    public func refresh() {
        root = GroupNode.tree(from: core.groups())
        // A group can go while it is selected, if someone deletes the folder.
        if selectedGroup != [], root?.find(selectedGroup) == nil {
            selectedGroup = []
        }
        reloadRows()
        guard let editing else { return }
        if editing.isDirty {
            // The draft on screen is the user's work: a change underneath it
            // never overwrites what they have typed. Only the advice is
            // refreshed, since a conflict may have arrived with the change.
            problems = core.checkDraft(draft: editing.draft, editing: editing.savedID)
        } else if let detail = core.snippet(id: editing.id) {
            self.editing = Editing(detail)
        } else {
            self.editing = nil
        }
    }

    /// What the body being edited would expand to. The file is not consulted,
    /// so the preview keeps up with the keystroke.
    public func preview(of body: String) -> String {
        core.previewDraft(body: body)
    }

    /// The core's change events. The window hands them straight over.
    public func libraryChanged(_ event: LibraryEvent) {
        if case .failed(let message) = event {
            problem = message
        } else {
            problem = nil
            refresh()
        }
    }

    private func reloadRows() {
        let hits = core.search(
            query: SearchQuery(
                text: query,
                group: selectedGroup.isEmpty ? nil : selectedGroup,
                tag: nil,
                enabledOnly: false,
                limit: 0
            )
        )
        rows = hits.map(SnippetRow.init)
    }

    // MARK: - Editing snippets

    /// Writes a new snippet into the selected group and opens it. Its
    /// abbreviation is suggested from the label and is free when it is offered.
    @discardableResult
    public func newSnippet(label: String = "New Snippet") throws -> String {
        let suggestion = core.suggestAbbreviation(label: label)
        let draft = SnippetDraft(
            label: label,
            abbreviations: suggestion.isEmpty ? [] : [suggestion],
            body: "",
            tags: [],
            kind: .text,
            trigger: nil,
            case: nil,
            wholeWord: nil,
            keepDelimiter: nil,
            enabled: nil
        )
        let id = try core.createSnippet(group: selectedGroup, draft: draft)
        refresh()
        select(snippet: id)
        return id
    }

    /// Writes the draft on screen to its file. A save is never blocked by
    /// `problems`: the folder is the user's.
    public func save() throws {
        guard let editing, editing.isDirty else { return }
        let id = try core.saveSnippet(id: editing.id, draft: editing.draft)
        refresh()
        select(snippet: id)
    }

    /// Throws away the edits on screen and shows the file again.
    public func revert() {
        guard let editing else { return }
        select(snippet: editing.id)
    }

    public func delete(snippet id: String) throws {
        try core.deleteSnippet(id: id)
        if editing?.id == id { editing = nil }
        refresh()
    }

    public func move(snippet id: String, to group: [String]) throws {
        try core.moveSnippet(id: id, group: group)
        refresh()
        if editing?.id == id { select(snippet: id) }
    }

    public func setEnabled(_ enabled: Bool, snippet id: String) throws {
        try core.setSnippetEnabled(id: id, enabled: enabled)
        refresh()
    }

    // MARK: - Editing groups

    /// Creates an empty group inside `parent` under a name nothing there uses
    /// yet, and selects it.
    @discardableResult
    public func newGroup(named name: String = "New Group", under parent: [String]) throws -> [String] {
        let taken = Set((root?.find(parent)?.children ?? []).map { $0.name.lowercased() })
        var chosen = name
        var next = 2
        while taken.contains(chosen.lowercased()) {
            chosen = "\(name) \(next)"
            next += 1
        }
        let path = parent + [chosen]
        try core.createGroup(group: path)
        refresh()
        select(group: path)
        return path
    }

    /// Renames a group's folder. The selection follows it.
    @discardableResult
    public func rename(group path: [String], to name: String) throws -> [String] {
        let moved = try core.renameGroup(group: path, name: name)
        follow(path, to: moved)
        return moved
    }

    @discardableResult
    public func move(group path: [String], under parent: [String]) throws -> [String] {
        let moved = try core.moveGroup(group: path, into: parent)
        follow(path, to: moved)
        return moved
    }

    /// Removes a group and everything in it. `contents(of:)` says how much
    /// that is, so the window can ask first.
    @discardableResult
    public func delete(group path: [String]) throws -> [String] {
        let removed = try core.deleteGroup(group: path)
        if selectedGroup.starts(with: path) { selectedGroup = [] }
        if let open = editing?.group, open.starts(with: path) { editing = nil }
        refresh()
        return removed
    }

    /// Switches a group on or off. Off is sticky: nothing inside it expands,
    /// whatever a group further down says.
    public func setEnabled(_ enabled: Bool, group path: [String]) throws {
        try core.setGroupEnabled(group: path, enabled: enabled)
        refresh()
    }

    public func setAppearance(group path: [String], colour: String?, icon: String?) throws {
        try core.setGroupAppearance(group: path, colour: colour, icon: icon)
        refresh()
    }

    /// How many snippets a group holds, counting the groups inside it.
    public func contents(of path: [String]) -> Int {
        Int(core.groupContents(group: path))
    }

    /// A group that moved takes the selection and the open snippet with it,
    /// so neither is left pointing at a folder that is no longer there.
    private func follow(_ was: [String], to now: [String]) {
        if selectedGroup.starts(with: was) {
            selectedGroup = now + selectedGroup.dropFirst(was.count)
        }
        refresh()
        if let id = editing?.id { select(snippet: id) }
    }
}
