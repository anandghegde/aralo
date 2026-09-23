import AraloBridge
import Foundation
import Observation

/// One sync conflict, put in front of the user: the two versions, what they
/// changed differently, and the three ways out of it. It is the resolver
/// apart from its window, so a second shell draws the window and nothing
/// else.
///
/// Whichever way the user goes, the copy ends up in the Trash and the
/// original's file holds what they chose.
@MainActor
@Observable
public final class ConflictResolver: Identifiable {
    public let detail: ConflictDetail
    /// The version the user is writing, when neither side will do. It starts
    /// as this Mac's.
    public var edited: String
    /// Why the last attempt failed, in the core's words.
    public private(set) var failure: String?

    @ObservationIgnored private let core: Core
    @ObservationIgnored private let resolved: () -> Void

    public nonisolated var id: String { detail.copy }

    init(core: Core, detail: ConflictDetail, resolved: @escaping () -> Void) {
        self.core = core
        self.detail = detail
        self.edited = detail.originalText
        self.resolved = resolved
    }

    /// The original's file name, as the Finder shows it.
    public var originalName: String { (detail.original as NSString).lastPathComponent }
    /// The copy's file name, which says which sync client made it.
    public var copyName: String { (detail.copy as NSString).lastPathComponent }

    /// Whether the copy can be kept. One that is not a snippet file cannot.
    public var copyIsReadable: Bool { detail.copyProblem == nil }

    /// What the two sides changed differently, in words.
    public var summary: String {
        if let problem = detail.copyProblem {
            return "The copy is not a snippet Aralo can read: \(problem)"
        }
        var parts = detail.clashingKeys.map(Self.describe)
        if detail.bodyClashes { parts.append("the text") }
        guard !parts.isEmpty else {
            return "The two versions differ, and there is no earlier version on this Mac to tell which change is newer."
        }
        return "Both versions changed \(Self.list(parts)), differently."
    }

    public func keepOriginal() -> Bool { resolve(.keepOriginal) }
    public func keepCopy() -> Bool { resolve(.keepCopy) }
    public func keepEdited() -> Bool { resolve(.write(text: edited)) }

    /// Settles the conflict. False, with `failure` set, when that did not
    /// work; nothing has moved then.
    private func resolve(_ choice: ConflictChoice) -> Bool {
        do {
            try core.resolveConflict(copy: detail.copy, choice: choice)
            failure = nil
            resolved()
            return true
        } catch {
            failure = error.reason
            return false
        }
    }

    /// A front-matter key as the snippet window names it.
    static func describe(_ key: String) -> String {
        keyNames[key] ?? "\u{201C}\(key)\u{201D}"
    }

    private static let keyNames: [String: String] = [
        "label": "the name",
        "abbr": "the abbreviations",
        "type": "the kind of snippet",
        "trigger": "when it expands",
        "case": "how it matches case",
        "word": "the whole-word setting",
        "keep_delimiter": "the delimiter setting",
        "enabled": "whether it is on",
        "tags": "the tags",
        "ai": "the AI settings"
    ]

    static func list(_ parts: [String]) -> String {
        switch parts.count {
        case 0: return ""
        case 1: return parts[0]
        default: return parts.dropLast().joined(separator: ", ") + " and " + parts[parts.count - 1]
        }
    }
}
