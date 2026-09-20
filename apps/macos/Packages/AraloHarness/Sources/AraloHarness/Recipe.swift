import Foundation

/// How to get a text field with the keyboard in one app. The recipes live in
/// `data/compat/matrix.json`, so fixing one at the matrix Mac needs no rebuild.
public struct Recipe: Codable, Equatable, Sendable {
    public enum Open: String, Codable, Sendable {
        /// Open an empty scratch file with the app.
        case document
        /// Open a scratch page whose text area takes the focus.
        case webPage = "web-page"
        /// Open a scratch script that runs `cat`, so typed lines go nowhere.
        case shell
        /// Bring the app forward and press `keys`.
        case keys
        /// Bring the app forward and wait for a person to click into a field.
        case manual
    }

    public enum Clear: String, Codable, Sendable {
        case selectAll = "select-all"
        case killLine = "kill-line"
    }

    public var bundleID: String
    public var open: Open
    /// For `document`: the scratch file's extension.
    public var fileExtension: String?
    /// For `keys`: what to press once the app is in front.
    public var keys: [String]?
    public var clear: Clear?
    /// How long the app gets after it comes forward, before the first key.
    public var settleMs: Int?
    public var note: String?

    enum CodingKeys: String, CodingKey {
        case bundleID = "bundle_id"
        case open
        case fileExtension = "extension"
        case keys
        case clear
        case settleMs = "settle_ms"
        case note
    }

    public init(bundleID: String, open: Open, clear: Clear? = nil, keys: [String]? = nil) {
        self.bundleID = bundleID
        self.open = open
        self.clear = clear
        self.keys = keys
    }

    public var clearing: Clear { clear ?? .selectAll }
    public var settle: TimeInterval { TimeInterval(settleMs ?? 1500) / 1000 }
    public var chords: [KeyChord] { (keys ?? []).compactMap(KeyChord.init) }

    /// The chords that empty the field.
    public var clearChords: [KeyChord] {
        switch clearing {
        case .selectAll: [.selectAll, .delete]
        case .killLine: [.killLine]
        }
    }
}

public struct RecipeBook: Codable, Equatable, Sendable {
    public static let supportedVersion = 0

    public var version: Int
    public var recipes: [Recipe]

    public enum Problem: Error, Equatable, CustomStringConvertible {
        case newerVersion(Int)
        case badKey(bundleID: String, key: String)
        case duplicate(String)

        public var description: String {
            switch self {
            case .newerVersion(let found): "the recipe file is version \(found); this harness reads \(supportedVersion)"
            case .badKey(let bundleID, let key): "\(bundleID): \"\(key)\" is not a key chord"
            case .duplicate(let bundleID): "\(bundleID) has two recipes"
            }
        }
    }

    public static func load(_ url: URL) throws -> RecipeBook {
        try parse(Data(contentsOf: url))
    }

    public static func parse(_ data: Data) throws -> RecipeBook {
        let book = try JSONDecoder().decode(RecipeBook.self, from: data)
        guard book.version <= supportedVersion else { throw Problem.newerVersion(book.version) }
        var seen: Set<String> = []
        for recipe in book.recipes {
            guard seen.insert(recipe.bundleID.lowercased()).inserted else { throw Problem.duplicate(recipe.bundleID) }
            if let bad = (recipe.keys ?? []).first(where: { KeyChord($0) == nil }) {
                throw Problem.badKey(bundleID: recipe.bundleID, key: bad)
            }
        }
        return book
    }

    /// An app without a recipe needs a person.
    public func recipe(for bundleID: String) -> Recipe {
        recipes.first { $0.bundleID.caseInsensitiveCompare(bundleID) == .orderedSame }
            ?? Recipe(bundleID: bundleID, open: .manual)
    }
}
