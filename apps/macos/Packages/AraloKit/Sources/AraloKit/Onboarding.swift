import AraloBridge
import Foundation

/// The first run, in the order the plan gives it (sections 4.5 and 8, task
/// 5.4): what Aralo reads and never stores, the two permissions, the library
/// and a way to bring snippets in, AI if the user wants it, then a field to
/// try it in.
public enum OnboardingStep: Int, CaseIterable, Comparable, Sendable {
    case privacy
    case accessibility
    case inputMonitoring
    case library
    case aiOptIn
    case tryIt

    public static func < (lhs: Self, rhs: Self) -> Bool { lhs.rawValue < rhs.rawValue }

    /// A screen that waits for a grant, and is skipped when it is there.
    public var isPermission: Bool { self == .accessibility || self == .inputMonitoring }

    /// A screen that sets Aralo up once. Someone coming back because a grant
    /// was taken away has done these already.
    public var isSetUp: Bool { self == .library || self == .aiOptIn }
}

/// Which screen the first run shows and when it may move on. The window only
/// draws what this says.
public struct OnboardingFlow: Equatable, Sendable {
    /// What the flow needs to know about the world, polled by the window.
    public struct Facts: Equatable, Sendable {
        public var permissions: Permissions.Status
        /// The tap is up and receives keys. With that true, a missing Input
        /// Monitoring grant is not worth stopping anyone for.
        public var tapRunning: Bool

        public init(permissions: Permissions.Status, tapRunning: Bool) {
            self.permissions = permissions
            self.tapRunning = tapRunning
        }

        func satisfies(_ step: OnboardingStep) -> Bool {
            switch step {
            case .privacy, .library, .aiOptIn, .tryIt: true
            case .accessibility: permissions.accessibility
            case .inputMonitoring: permissions.inputMonitoring || tapRunning
            }
        }
    }

    public private(set) var step: OnboardingStep
    /// The screens this run shows, in order. A returning user is not walked
    /// through the set-up screens again.
    public let steps: [OnboardingStep]

    public init(startingAt step: OnboardingStep = .privacy, returning: Bool = false) {
        self.step = step
        steps = OnboardingStep.allCases.filter { !returning || !$0.isSetUp }
    }

    /// Where `step` is in `steps`, from one, for "Step 2 of 6".
    public var position: Int { (steps.firstIndex(of: step) ?? 0) + 1 }

    /// Where to open the flow at launch or from the menu: at the start for a
    /// new user, at whatever was revoked for a returning one. Nil when there
    /// is nothing to set up.
    public static func entryPoint(completedBefore: Bool, facts: Facts) -> OnboardingStep? {
        guard completedBefore else { return .privacy }
        return [.accessibility, .inputMonitoring].first { !facts.satisfies($0) }
    }

    public func canContinue(_ facts: Facts) -> Bool {
        facts.satisfies(step)
    }

    public var isLast: Bool { step == .tryIt }

    /// Moves to the next screen that still has something to ask. Returns
    /// false when the flow is over.
    @discardableResult
    public mutating func advance(_ facts: Facts) -> Bool {
        guard canContinue(facts), !isLast else { return false }
        let later = steps.filter { $0 > step }
        step = later.first { !$0.isPermission || !facts.satisfies($0) } ?? .tryIt
        return true
    }

    public mutating func back() {
        step = steps.last { $0 < step } ?? steps.first ?? .privacy
    }
}

/// The abbreviation the test field asks for, from the user's own library.
public struct OnboardingExample: Equatable, Sendable {
    public var abbreviation: String
    /// The start of what it expands to.
    public var expansion: String

    public init(abbreviation: String, expansion: String) {
        self.abbreviation = abbreviation
        self.expansion = expansion
    }

    /// The starter set's `ty` when it is there, because it is the one the
    /// README promises. Otherwise any enabled snippet that can be recognised
    /// afterwards: plain text on one line.
    public static func pick(from snippets: [SnippetSummary]) -> OnboardingExample? {
        let usable = snippets.lazy.filter(\.enabled).compactMap { snippet -> OnboardingExample? in
            let expansion = snippet.preview.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let abbreviation = snippet.abbreviations.first, !abbreviation.isEmpty,
                  !expansion.isEmpty, !expansion.contains("{{"), !expansion.contains(where: \.isNewline)
            else { return nil }
            return OnboardingExample(abbreviation: abbreviation, expansion: expansion)
        }
        return usable.first { $0.abbreviation == "ty" } ?? usable.first
    }

    /// Whether the field shows the expansion. Case is ignored: "Ty" expands
    /// to "Thank you".
    public func isExpanded(in text: String) -> Bool {
        text.range(of: expansion, options: .caseInsensitive) != nil
    }
}

/// What the library screen says about the folder the snippets are in: whose
/// they are, how many, and where.
public struct OnboardingLibrary: Equatable, Sendable {
    public var snippetCount: Int
    /// How many starter files Aralo wrote as it opened the folder. Nonzero
    /// only on the run that made the library.
    public var starterFiles: Int
    public var folder: URL

    public init(snippetCount: Int, starterFiles: Int, folder: URL) {
        self.snippetCount = snippetCount
        self.starterFiles = starterFiles
        self.folder = folder
    }

    /// One sentence on what is in the library.
    public var summary: String {
        if snippetCount == 0 {
            return "Your library is empty. Import snippets from another app, or write your first one later."
        }
        let snippets = snippetCount == 1 ? "1 snippet" : "\(snippetCount) snippets"
        if starterFiles > 0 {
            return "Aralo made your library and put \(snippets) in it to start with."
        }
        return "Your library already has \(snippets)."
    }

    /// The folder as a person would say it: in the home folder, with a tilde.
    public var displayPath: String {
        let home = FileManager.default.homeDirectoryForCurrentUser.standardizedFileURL.path
        let path = folder.standardizedFileURL.path
        guard path == home || path.hasPrefix(home + "/") else { return path }
        return "~" + path.dropFirst(home.count)
    }
}

@MainActor
public extension AraloService {
    /// What the first run's library screen says: how many snippets, and
    /// whether Aralo wrote them because the folder was new.
    var onboardingLibrary: OnboardingLibrary {
        OnboardingLibrary(
            snippetCount: snippetCount, starterFiles: Int(core?.starterFilesWritten() ?? 0), folder: libraryURL
        )
    }
}
