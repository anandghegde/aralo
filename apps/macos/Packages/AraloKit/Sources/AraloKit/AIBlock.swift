import AraloBridge
import Foundation

/// One `{{ai}}` block of a snippet, as the AI preview shows it (PRD A2): what
/// it asks, what the model has written so far, and what goes in if no model
/// answers.
public struct AIBlock: Identifiable, Equatable {
    public enum Status: Equatable, Sendable {
        /// Not asked yet: another block is being written first.
        case waiting
        /// The answer is streaming in.
        case writing
        /// The answer is in, or the user wrote their own. `text` goes in.
        case written
        /// No model answered, and why: the core's words, or the panel's in
        /// the same style. The fallback goes in.
        case failed(String)
    }

    /// What the core knows the block by.
    public let index: UInt32
    /// What the model is asked to write: the snippet's own words.
    public let prompt: String
    /// What goes in when no model answers. Nil puts in nothing.
    public let fallback: String?
    public internal(set) var status: Status = .waiting
    /// What goes in: the model's answer as it arrives, fitted once it is in,
    /// or the user's edit of it.
    public internal(set) var text = ""
    /// Who answered, and what was sent to them.
    public internal(set) var profile: String?
    public internal(set) var model: String?
    public internal(set) var sent: [AiContextSent] = []
    /// The model stopped at its length limit, or was stopped: the end may be
    /// missing.
    public internal(set) var cutShort = false

    public var id: UInt32 { index }

    init(_ info: AiBlockInfo) {
        index = info.index
        prompt = info.prompt
        fallback = info.fallback
    }
}

/// What asks a model for a block: the AI settings in the app, and a fake in
/// the tests.
public protocol BlockRunner: Sendable {
    func start(_ session: ExpansionSession, block: UInt32) async throws -> AiBlockRunProtocol
}

extension AiProfiles: BlockRunner {
    public func start(_ session: ExpansionSession, block: UInt32) async throws -> AiBlockRunProtocol {
        try await runBlock(session: session, index: block)
    }
}

/// Opens the AI settings the first time a block asks, so a snippet with no
/// AI block never causes profiles.toml or the keychain to be read.
public struct DeferredBlockRunner: BlockRunner {
    private let settings: @MainActor @Sendable () throws -> AiProfiles

    public init(settings: @escaping @MainActor @Sendable () throws -> AiProfiles) {
        self.settings = settings
    }

    public func start(_ session: ExpansionSession, block: UInt32) async throws -> AiBlockRunProtocol {
        let profiles = try await settings()
        return try await profiles.start(session, block: block)
    }
}

/// What a request sent and to whom, in one line: the privacy promise where the
/// user reads the answer. The command panel and the AI preview both say it.
///
/// A kind the snippet declared but the app had nothing for was not sent, and
/// is not named. `alone` is what went when no context did.
public func sentSummary(
    _ sent: [AiContextSent], profile: String?, model: String?, alone: String = "the prompt only"
) -> String {
    guard let profile else { return "" }
    let parts = sent.compactMap { sent -> String? in
        guard let bytes = sent.bytes else { return nil }
        let name = switch sent.kind {
        case .selection: "the selection"
        case .clipboard: "the clipboard"
        case .fillins: "the answers you gave"
        case .app: "the app's name"
        case .window: "the window title"
        }
        return "\(name) (\(ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)))"
    }
    let what = parts.isEmpty ? alone : parts.joined(separator: ", ")
    return "Sent \(what) to \(profile) · \(model ?? "")"
}

/// The stretches of `text` that the spans mark, as ranges of the string. The
/// core counts in UTF-16 code units, as a text view does; a span that does not
/// land on characters of `text` is left out rather than trusted.
public func markedRanges(in text: String, spans: [BlockSpan]) -> [Range<String.Index>] {
    let units = text.utf16
    return spans.compactMap { span in
        guard let first = units.index(units.startIndex, offsetBy: Int(span.start), limitedBy: units.endIndex),
              let last = units.index(first, offsetBy: Int(span.length), limitedBy: units.endIndex),
              let lower = first.samePosition(in: text),
              let upper = last.samePosition(in: text)
        else { return nil }
        return lower..<upper
    }
}
