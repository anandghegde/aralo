import AraloBridge
import Foundation

/// One group in the sidebar, with the groups inside it.
public struct GroupNode: Identifiable, Equatable, Sendable {
    /// Folder names from the library root down. Empty for the root itself.
    public var path: [String]
    public var name: String
    public var colour: String?
    public var icon: String?
    /// False when this group or any group above it is switched off.
    public var enabled: Bool
    /// Snippets directly in this group.
    public var snippets: Int
    /// Snippets here and in every group below it: what the list will show.
    public var total: Int
    public var children: [GroupNode]

    public var id: String { path.joined(separator: "/") }
    public var isRoot: Bool { path.isEmpty }

    /// The tree the flat list of groups describes. Nil for a library that has
    /// not loaded, which has not even a root.
    public static func tree(from groups: [GroupSummary]) -> GroupNode? {
        var byParent: [String: [GroupSummary]] = [:]
        var root: GroupSummary?
        for group in groups {
            if group.path.isEmpty {
                root = group
            } else {
                byParent[group.path.dropLast().joined(separator: "/"), default: []].append(group)
            }
        }
        return root.map { build($0, byParent) }
    }

    private static func build(_ summary: GroupSummary, _ byParent: [String: [GroupSummary]]) -> GroupNode {
        let children = (byParent[summary.path.joined(separator: "/")] ?? [])
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
            .map { build($0, byParent) }
        let own = Int(summary.snippets)
        return GroupNode(
            path: summary.path,
            name: summary.name,
            colour: summary.colour,
            icon: summary.icon,
            enabled: summary.enabled,
            snippets: own,
            total: own + children.reduce(0) { $0 + $1.total },
            children: children
        )
    }

    /// The path an `id` names. A folder name never holds a separator, so the
    /// two are the same thing written differently.
    public static func path(of id: String) -> [String] {
        id.isEmpty ? [] : id.components(separatedBy: "/")
    }

    /// The group at `path`, at any depth. Nil when nothing is there.
    public func find(_ path: [String]) -> GroupNode? {
        if path == self.path { return self }
        guard path.starts(with: self.path) else { return nil }
        for child in children {
            if let found = child.find(path) { return found }
        }
        return nil
    }

    /// The groups inside this one, or nil when there are none, so a sidebar
    /// row with nothing under it draws no disclosure triangle.
    public var subgroups: [GroupNode]? { children.isEmpty ? nil : children }

    /// This group and every group below it, depth first: a flat sidebar, or
    /// the list of places a snippet can be moved to.
    public var flattened: [GroupNode] {
        [self] + children.flatMap(\.flattened)
    }
}

/// One row of the snippet list.
public struct SnippetRow: Identifiable, Equatable, Sendable {
    public var id: String
    public var label: String
    public var abbreviations: [String]
    public var group: [String]
    /// False when the snippet or a group above it is switched off.
    public var enabled: Bool
    /// The text the search matched: an abbreviation, the label, a tag, the
    /// group, or the body line the match sits on. For an empty search it is
    /// the label, and the row shows the group instead.
    public var detail: String
    /// Which field `detail` came from.
    public var field: SearchField
    /// Character offsets into `detail` that matched, for the row to highlight.
    /// Empty when nothing was searched for.
    public var matched: [Int]

    init(_ hit: SearchResult) {
        id = hit.id
        label = hit.name
        abbreviations = hit.abbreviations
        group = hit.group
        enabled = hit.enabled
        detail = hit.text
        field = hit.field
        matched = hit.matched.map(Int.init)
    }
}

/// A snippet open in the editor: what is on disk, and what has been typed over
/// it.
public struct Editing: Equatable, Sendable {
    /// The snippet's ID. It survives a rename or a move.
    public var id: String
    /// Path from the library root, for the window to show and for "Reveal".
    public var path: String
    public var group: [String]
    /// The fields as they are being edited.
    public var draft: SnippetDraft
    /// The fields as the file has them.
    public var saved: SnippetDraft
    /// What the draft's inherited fields come to once the groups above are
    /// applied: what the editor shows greyed behind an empty choice.
    public var resolved: ResolvedSettings
    /// What typing the abbreviation would produce, as the file stands.
    public var preview: String

    init(_ detail: SnippetDetail) {
        id = detail.id
        path = detail.path
        group = detail.group
        draft = detail.draft
        saved = detail.draft
        resolved = detail.resolved
        preview = detail.preview
    }

    public var isDirty: Bool { draft != saved }

    /// The ID to check a draft against, so a snippet's own abbreviations are
    /// not reported as taken by itself.
    var savedID: String { id }
}
