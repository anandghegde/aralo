import Foundation

/// What the matrix tries in every app (the implementation plan, section 11).
public enum MatrixCase: String, CaseIterable, Codable, Sendable {
    case ascii
    case unicode
    case long
    case cursor
    case undo
    case clipboard
    case form

    /// The abbreviation to type, from `fixtures/matrix/library`. Lower-case
    /// letters only, so every Latin keyboard layout has them on unshifted keys.
    public var abbreviation: String {
        switch self {
        case .ascii, .undo: "mxascii"
        case .unicode: "mxemoji"
        case .long, .clipboard: "mxlong"
        case .cursor: "mxcursor"
        case .form: "mxform"
        }
    }

    /// Why the case cannot pass yet. Nothing waits on a later milestone now
    /// that the evaluator and the form panel are in; a case added ahead of what
    /// it tests says so here and is skipped unless `--include-pending`.
    public var pendingReason: String? {
        nil
    }

    /// Whether a form panel opens and has to be answered before anything is
    /// inserted. The panel is Aralo's own window, so the app under test stays
    /// frontmost and the answer is typed with the same keyboard.
    public var answersAForm: Bool {
        self == .form
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
