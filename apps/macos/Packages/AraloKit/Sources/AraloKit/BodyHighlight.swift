import AraloBridge
import Foundation

/// How one run of the body should be drawn.
public enum BodyStyle: Equatable, Sendable {
    /// A `{{…}}` the core recognises.
    case placeholder
    /// A `{{…}}` that reads as one but that no placeholder is called. It
    /// expands as the text it is written as; `BodyAdvice` says so.
    case unknownPlaceholder
    /// Something the core could not read. Underlined.
    case error
    /// Advice about a range that still reads. Underlined faintly.
    case note
}

/// One range of the body and how to draw it.
public struct BodyRun: Equatable, Sendable {
    /// UTF-16 code units, which is what a text view counts in.
    public var range: NSRange
    public var style: BodyStyle

    public init(range: NSRange, style: BodyStyle) {
        self.range = range
        self.style = style
    }
}

/// One thing to say under the body, and the range it is about.
public struct BodyAdvice: Identifiable, Equatable, Sendable {
    /// True for something the core could not read, false for advice.
    public var isError: Bool
    /// One sentence, in the core's words, so every shell says the same thing
    /// about the same body.
    public var message: String
    /// What to select when the reader clicks it.
    public var range: NSRange

    public var id: String { "\(range.location)-\(range.length)-\(message)" }

    public init(isError: Bool, message: String, range: NSRange) {
        self.isError = isError
        self.message = message
        self.range = range
    }
}

/// What an editor draws over a body: the runs to colour and underline, and the
/// sentences to show beneath it.
///
/// The core does the reading; this only turns its offsets into `NSRange`s and
/// keeps them inside the text the view actually holds. An outline read a
/// keystroke ago must never make a text view throw, so anything the body no
/// longer covers is clamped or dropped.
public struct BodyHighlight: Equatable, Sendable {
    /// In drawing order: the placeholders first, then the problems over them,
    /// so an underline lands on top of the colour it belongs to.
    public var runs: [BodyRun]
    /// First problem first. A message is kept even when its range has gone,
    /// because the reader still needs to be told.
    public var advice: [BodyAdvice]

    /// A body with nothing to draw over it.
    public static let none = BodyHighlight(runs: [], advice: [])

    public init(runs: [BodyRun], advice: [BodyAdvice]) {
        self.runs = runs
        self.advice = advice
    }

    /// Turns the core's reading of `body` into what a text view draws over it.
    public init(outline: BodyOutline, body: String) {
        let length = body.utf16.count
        var runs: [BodyRun] = []
        for placeholder in outline.placeholders {
            guard let range = Self.clamp(placeholder.start, placeholder.end, to: length) else { continue }
            runs.append(BodyRun(range: range, style: placeholder.known ? .placeholder : .unknownPlaceholder))
        }
        var advice: [BodyAdvice] = []
        for problem in outline.problems {
            let isError = problem.level == .error
            let range = Self.clamp(problem.start, problem.end, to: length)
            if let range {
                runs.append(BodyRun(range: range, style: isError ? .error : .note))
            }
            advice.append(
                BodyAdvice(
                    isError: isError,
                    message: problem.message,
                    range: range ?? NSRange(location: min(Int(problem.start), length), length: 0)
                )
            )
        }
        self.init(runs: runs, advice: advice)
    }

    public var isEmpty: Bool { runs.isEmpty && advice.isEmpty }

    /// The range to draw, or nil when there is nothing left of it: a range the
    /// body has grown past, or one that was empty to begin with.
    private static func clamp(_ start: UInt32, _ end: UInt32, to length: Int) -> NSRange? {
        let first = Int(start)
        let last = min(Int(end), length)
        guard first < last else { return nil }
        return NSRange(location: first, length: last - first)
    }
}

/// One placeholder an editor's insert menu offers. The list is the core's, so
/// two shells offer the same placeholders in the same words.
public struct Placeholder: Identifiable, Equatable, Sendable {
    public var name: String
    /// One line for the menu item.
    public var summary: String
    /// The text to put in at the caret.
    public var insert: String
    /// The part of `insert` to select once it is in, in UTF-16 code units from
    /// the start of the insertion: the words the reader replaces with their
    /// own. An empty range leaves the caret after the insertion.
    public var select: NSRange
    /// False while the core inserts the placeholder as written instead of
    /// working out what it means. Nothing is evaluated before the evaluator
    /// lands, so the preview shows the source text.
    public var expands: Bool

    public var id: String { name }

    init(_ choice: PlaceholderChoice) {
        name = choice.name
        summary = choice.summary
        insert = choice.insert
        select = NSRange(
            location: Int(choice.selectStart),
            length: Int(choice.selectEnd) - Int(choice.selectStart)
        )
        expands = choice.evaluated
    }

    /// Every placeholder the format defines, in the order to offer them. The
    /// list cannot change while the app runs, so it is read from the core once.
    public static let all: [Placeholder] = placeholderChoices().map(Placeholder.init)

    /// What to select once `insert` has gone in at `caret`, in the text view's
    /// own offsets.
    public func selection(insertedAt caret: Int) -> NSRange {
        NSRange(location: caret + select.location, length: select.length)
    }
}
