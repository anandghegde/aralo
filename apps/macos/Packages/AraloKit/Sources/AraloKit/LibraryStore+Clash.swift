import AraloBridge
import Foundation

// Leaving a draft, and the ways out of a save that clashed with a change made
// on disk while the editor had the snippet open (plan 5.1).
extension LibraryStore {
    /// Saves what is on screen before the window moves off it. False when the
    /// save clashed with a change on disk: the draft is still the only copy of
    /// the user's work, so the window stays where it is until they decide.
    @discardableResult
    public func leave() -> Bool {
        attempt { try $0.save() }
        return clash == nil
    }

    /// Settles a clash by saving the draft over the file as it now is. Where
    /// the two disagree, the draft wins; anything else on disk stays.
    public func keepMine() throws {
        guard let found = clash, var editing else { return }
        clash = nil
        guard let disk = found.disk else {
            // Nothing on disk to merge with: the draft is the snippet.
            let id = try core.saveSnippet(id: editing.id, draft: editing.draft)
            refresh()
            select(snippet: id)
            return
        }
        editing.saved = disk
        self.editing = editing
        try save()
        if clash == nil, self.editing?.isDirty == false {
            select(snippet: editing.id)
        }
    }

    /// Settles a clash by dropping the draft and showing the file as it is on
    /// disk.
    public func useTheirs() throws {
        guard let found = clash else { return }
        // The folder may not have been read since the file changed.
        try core.reload()
        refresh()
        select(snippet: found.id)
    }

    /// Keeps the draft on screen and makes the file as it now is what the
    /// next save goes over, so the user can fold in what they want from it
    /// first. Nothing is written.
    public func editBeforeSaving() {
        guard let found = clash else { return }
        clash = nil
        if let disk = found.disk { editing?.saved = disk }
    }

    /// Puts a clash away undecided. The draft stays as it was, and the next
    /// save finds the same clash.
    public func dismissClash() {
        clash = nil
    }
}
