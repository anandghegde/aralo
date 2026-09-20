import AraloBridge
import Foundation

/// The first run, in the order the plan gives it (section 4.5): what Aralo
/// reads and never stores, the two permissions, then a field to try it in.
public enum OnboardingStep: Int, CaseIterable, Comparable, Sendable {
    case privacy
    case accessibility
    case inputMonitoring
    case tryIt

    public static func < (lhs: Self, rhs: Self) -> Bool { lhs.rawValue < rhs.rawValue }
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
            case .privacy, .tryIt: true
            case .accessibility: permissions.accessibility
            case .inputMonitoring: permissions.inputMonitoring || tapRunning
            }
        }
    }

    public private(set) var step: OnboardingStep

    public init(startingAt step: OnboardingStep = .privacy) {
        self.step = step
    }

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
        let later = OnboardingStep.allCases.filter { $0 > step }
        step = later.first { $0 == .tryIt || !facts.satisfies($0) } ?? .tryIt
        return true
    }

    public mutating func back() {
        step = OnboardingStep(rawValue: step.rawValue - 1) ?? .privacy
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
