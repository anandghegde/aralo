import AraloBridge
import Foundation
import Observation

public extension AraloService {
    /// Moves the library to `destination`, an empty folder or one that does
    /// not exist yet, and runs it from there (plan 5.2). The copy is checked
    /// before anything switches, off the main thread; expansion goes on from
    /// the old folder until it does. The old folder is left where it was.
    func moveLibrary(to destination: URL) async throws -> LibraryMove {
        guard let core else { throw BridgeError.Library(message: "The library is not open.") }
        let path = destination.path
        let moved = try await Task.detached { try core.moveLibrary(to: path) }.value
        libraryMoved(to: URL(fileURLWithPath: moved.to, isDirectory: true))
        return moved
    }

    /// Runs the library that is already at `folder`, one another Mac synced
    /// there, say. Nothing is copied.
    func useLibrary(at folder: URL) async throws {
        guard let core else { throw BridgeError.Library(message: "The library is not open.") }
        let path = folder.path
        try await Task.detached { try core.switchLibrary(path: path) }.value
        libraryMoved(to: folder)
    }

    private func libraryMoved(to folder: URL) {
        libraryURL = folder
        UserDefaults.standard.set(folder.path, forKey: Self.libraryPathKey)
        onLibraryMoved?(folder)
    }
}

/// The Library tab of Settings: where the library is, which sync client
/// looks after it, and moving it or picking one another Mac synced.
///
/// The core decides what a folder is and does the move; this chooses between
/// moving and switching from what the core says is there, and puts the old
/// folder in the Trash only when nothing reached it after the copy.
@MainActor
@Observable
public final class LibraryLocationStore {
    public private(set) var location: LibraryLocation
    public private(set) var isMoving = false
    /// What the last move or switch did, in words.
    public private(set) var outcome: String?
    public private(set) var failure: String?
    /// The folder the library was moved out of, while it may go to the
    /// Trash. Nil when there is none, or when something in it changed after
    /// the copy and it has to stay for the user to look at.
    public private(set) var leftBehind: URL?

    private let service: AraloService

    public init(service: AraloService) {
        self.service = service
        location = inspectLibraryLocation(path: service.libraryURL.path)
    }

    /// Where a picked folder sends the library. A folder that already holds
    /// other files, iCloud Drive's top level say, gets an Aralo folder of its
    /// own inside it.
    public static func destination(for picked: URL) -> URL {
        guard inspectLibraryLocation(path: picked.path).contents == .occupied else { return picked }
        return picked.appendingPathComponent("Aralo", isDirectory: true)
    }

    /// Moves the library to `picked`, or runs the one that is already there.
    public func choose(_ picked: URL) async {
        let target = Self.destination(for: picked)
        failure = nil
        outcome = nil
        isMoving = true
        defer {
            isMoving = false
            location = inspectLibraryLocation(path: service.libraryURL.path)
        }
        do {
            switch inspectLibraryLocation(path: target.path).contents {
            case .empty:
                let moved = try await service.moveLibrary(to: target)
                if moved.changedAfterCopy.isEmpty {
                    leftBehind = URL(fileURLWithPath: moved.from, isDirectory: true)
                    outcome = "Moved \(moved.snippets) snippets. The old folder can go to the Trash."
                } else {
                    leftBehind = nil
                    outcome = "Moved. \(moved.changedAfterCopy.count) files in the old folder changed while "
                        + "it was copied, so it stays where it is: compare them before you delete it."
                }
                if let problem = moved.indexProblem {
                    outcome = (outcome ?? "") + " Recents and counts start again: \(problem)"
                }
            case .library(let snippets):
                try await service.useLibrary(at: target)
                leftBehind = nil
                outcome = "Using the library that was already there, with \(snippets) snippets."
            case .occupied:
                failure = "That folder already holds other files. Choose an empty one."
            }
        } catch {
            failure = error.reason
        }
    }

    /// Puts the folder the library moved out of in the Trash.
    public func trashLeftBehind() {
        guard let folder = leftBehind else { return }
        do {
            try FileManager.default.trashItem(at: folder, resultingItemURL: nil)
            leftBehind = nil
            outcome = "The old folder is in the Trash."
        } catch {
            failure = error.localizedDescription
        }
    }
}
