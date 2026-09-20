import Foundation

/// Which compatibility table Aralo runs with during a matrix run.
public enum CompatConfig: String, CaseIterable, Sendable {
    /// The table as shipped: every app with the method chosen for it. This is
    /// what the milestone's exit test counts.
    case chosen
    /// Every app typed into, whatever the table says.
    case type
    /// Every app pasted into.
    case paste

    /// The override table to hand Aralo through `ARALO_COMPAT`, or nil for the
    /// one built into the core. A forced method has no app entries: the point
    /// is to learn what each app does without its exceptions.
    public func table(undo: String = "native") -> String? {
        guard self != .chosen else { return nil }
        return """
            # Written by aralo-harness: forces one insert method on every app.
            version = 0

            [defaults]
            insert = "\(rawValue)"
            undo = "\(undo)"

            """
    }
}
