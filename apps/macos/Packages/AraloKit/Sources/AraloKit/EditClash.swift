import AraloBridge
import Foundation

/// A save that found the snippet's file changed on disk, by another Mac
/// through a sync client or by another editor, in the same place the draft
/// changed it (plan 5.1). Nothing was written: the draft stays on screen until
/// the user picks, through `LibraryStore.keepMine()`, `useTheirs()` or
/// `editBeforeSaving()`.
///
/// A change on disk that does not touch what the draft touched never gets
/// here; the save merges it in.
public struct EditClash: Identifiable, Equatable, Sendable {
    /// The snippet's ID.
    public let id: String
    /// The snippet's name, as the editor had it.
    public let name: String
    /// The file as the draft would write it.
    public let mineText: String
    /// The file as it is on disk now.
    public let diskText: String
    /// The file as it stood when the editor opened it.
    public let baseText: String
    /// The file on disk as the editor would open it. Nil when it no longer
    /// reads as a snippet.
    let disk: SnippetDraft?
    let clashingKeys: [String]
    let bodyClashes: Bool

    init(id: String, name: String, clash: SaveClash) {
        self.id = id
        self.name = name
        mineText = clash.mineText
        diskText = clash.diskText
        baseText = clash.baseText
        disk = clash.disk
        clashingKeys = clash.clashingKeys
        bodyClashes = clash.bodyClashes
    }

    /// Whether the file on disk can be taken as it is. One that no longer
    /// reads as a snippet cannot be put in the editor.
    public var diskIsReadable: Bool { disk != nil }

    /// What the two changed differently, in words.
    @MainActor public var summary: String {
        guard disk != nil else {
            return "The file on disk is no longer a snippet Aralo can read."
        }
        var parts = clashingKeys.map(ConflictResolver.describe)
        if bodyClashes { parts.append("the text") }
        return "Both changed \(ConflictResolver.list(parts)), differently."
    }
}
