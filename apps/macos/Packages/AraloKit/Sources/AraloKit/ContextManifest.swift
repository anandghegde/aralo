import AraloBridge
import Foundation

/// What one request sent, and to whom: the manifest a panel shows beside the
/// answer (plan 4.10, PRD P4).
///
/// It is the core's own record, taken after every stage that changes the
/// text, so each size is the bytes that went out. A kind the snippet or the
/// command declared but had nothing to give is listed too, as not sent: the
/// list is the whole of what the request was allowed to carry, and nothing
/// that is not on it left the Mac.
public struct ContextManifest: Equatable, Sendable {
    public struct Line: Equatable, Sendable, Identifiable {
        public let kind: AiContextKind
        /// The bytes sent. Nil when there was nothing to send.
        public let bytes: UInt64?

        public var id: AiContextKind { kind }

        /// What the line is about, as a sentence would begin it.
        public var name: String {
            switch kind {
            case .selection: "The selection"
            case .clipboard: "The clipboard"
            case .fillins: "The answers you gave"
            case .app: "The app's name"
            case .window: "The window title"
            }
        }

        /// How much went, or that none did.
        public var detail: String {
            guard let bytes else { return "nothing to send, so nothing sent" }
            return ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
        }
    }

    /// Every declared kind, in the order the request declared it.
    public let lines: [Line]
    /// Who answered. Nil before a request has gone out.
    public let profile: String?
    public let model: String?
    /// What the request is besides its context: "the prompt", "the
    /// instruction", "the label".
    public let request: String

    public init(_ sent: [AiContextSent], profile: String?, model: String?, request: String = "the prompt") {
        lines = sent.map { Line(kind: $0.kind, bytes: $0.bytes) }
        self.profile = profile
        self.model = model
        self.request = request
    }

    /// Whether a request has gone out, and there is anything to show.
    public var hasGoneOut: Bool { profile != nil }

    /// Where it went: the profile and the model.
    public var destination: String {
        [profile, model].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
    }

    /// One line for the panel's footer: what went, and where. Empty before a
    /// request has gone out. A kind with nothing to send is not named here;
    /// the full list says it.
    public var summary: String {
        guard hasGoneOut else { return "" }
        let parts = lines.compactMap { line -> String? in
            guard line.bytes != nil else { return nil }
            return "\(line.name.lowercasedFirst) (\(line.detail))"
        }
        let what = parts.isEmpty ? "\(request) only" : parts.joined(separator: ", ")
        return "Sent \(what) to \(destination)"
    }

    /// The list's last word: what went with the context, and that nothing
    /// else did.
    public var closing: String {
        "With \(request). Nothing else was sent."
    }
}

private extension String {
    var lowercasedFirst: String {
        guard let first else { return self }
        return first.lowercased() + dropFirst()
    }
}
