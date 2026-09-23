import AraloBridge
import Foundation

/// The core's `Trash`, as the Finder's. A conflict copy that has been merged
/// or resolved goes where a Mac user looks for something they want back, and
/// Put Back returns it to the library folder.
///
/// The core calls this with the library locked, on whichever thread read the
/// folder. Moving one file is all it does.
public final class SystemTrash: Trash, @unchecked Sendable {
    public init() {}

    public func discard(path: String) throws {
        do {
            try FileManager.default.trashItem(at: URL(fileURLWithPath: path), resultingItemURL: nil)
        } catch {
            throw BridgeError.Trash(message: error.localizedDescription)
        }
    }
}
