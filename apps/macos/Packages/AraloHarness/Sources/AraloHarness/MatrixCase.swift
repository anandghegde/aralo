import Foundation

/// What the matrix tries in every app (the implementation plan, section 11).
public enum MatrixCase: String, CaseIterable, Codable, Sendable {
    case ascii
    case unicode
    case long
    case cursor
    case undo
    case clipboard

    /// The abbreviation to type, from `fixtures/matrix/library`. Lower-case
    /// letters only, so every Latin keyboard layout has them on unshifted keys.
    public var abbreviation: String {
        switch self {
        case .ascii, .undo: "mxascii"
        case .unicode: "mxemoji"
        case .long, .clipboard: "mxlong"
        case .cursor: "mxcursor"
        }
    }

    /// Why the case cannot pass yet, for a case that waits on a later milestone.
    public var pendingReason: String? {
        self == .cursor ? "needs the template evaluator (M2)" : nil
    }

    /// Whether the time from the delimiter to the text counts as a latency sample.
    public var measuresLatency: Bool {
        self == .ascii || self == .unicode || self == .long
    }

    /// Long bodies are typed key by key in a terminal; give them longer.
    public var patience: Double {
        abbreviation == "mxlong" ? 3 : 1
    }
}
