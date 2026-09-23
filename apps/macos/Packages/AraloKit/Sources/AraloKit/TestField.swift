import AraloBridge
import Foundation
import Observation

/// The editor's test field: a draft typed out before it is saved.
///
/// The core holds the field and runs the same matcher and expansion path as
/// typing into any app, against the draft on screen rather than the file. The
/// text view forwards the keys typed into it and draws `field`; it never edits
/// its own text.
///
/// While the field has the keyboard, the live tap stands aside: otherwise an
/// abbreviation typed here would expand twice, once by the field and once by
/// the tap typing into it.
@MainActor
@Observable
public final class TestField {
    /// What the text view shows: the text, the caret and anything an expansion
    /// left selected, in UTF-16 like the view itself.
    public private(set) var field = TrialField(text: "", caret: 0, selected: 0)

    /// The form the draft asked for, while it is waiting to be filled in.
    /// Nothing else is typed into the field until it is over.
    public private(set) var form: FormSession?

    /// How many of the draft's abbreviations can be typed here. Zero means
    /// nothing will expand, which the editor says rather than leaving the user
    /// typing at a field that does nothing.
    public let abbreviations: UInt32

    @ObservationIgnored private let trial: DraftTrial
    @ObservationIgnored private let label: String
    @ObservationIgnored private let clipboard: @MainActor () -> String?
    @ObservationIgnored private let claimKeyboard: @MainActor (Bool) -> Void
    @ObservationIgnored private var viewHasKeyboard = false
    @ObservationIgnored private var formHasKeyboard = false
    @ObservationIgnored private var claimed = false

    init(
        trial: DraftTrial,
        label: String,
        clipboard: @escaping @MainActor () -> String?,
        claimKeyboard: @escaping @MainActor (Bool) -> Void
    ) {
        self.trial = trial
        self.label = label
        self.clipboard = clipboard
        self.claimKeyboard = claimKeyboard
        abbreviations = trial.abbreviations()
    }

    /// One key typed into the view. Ignored while a form is up: its boxes have
    /// the keyboard, and the key that opened it is already held by the session.
    public func type(_ key: KeyInput) {
        guard form == nil else { return }
        switch trial.key(key: key) {
        case .typed, .expanded:
            redraw()
        case .session(let session):
            let form = FormSession(
                session: session,
                runner: TrialRunner(trial: trial),
                label: label,
                clipboard: clipboard
            )
            // A body that only wanted the clipboard has expanded by now.
            if !form.isFinished {
                self.form = form
            }
            redraw()
        }
    }

    /// Everything a text field sends for one piece of typing, a character at a
    /// time, as a keyboard would have typed it.
    public func type(_ text: String) {
        for scalar in text.unicodeScalars {
            type(.char(scalar: Self.fromView(scalar).value))
        }
    }

    /// The user clicked or pressed an arrow key. What was typed before the
    /// caret no longer leads up to it. `caret` is in UTF-16 code units.
    public func moveCaret(to caret: Int) {
        trial.moveCaret(caret: UInt32(clamping: caret))
        redraw()
    }

    public func clear() {
        trial.clear()
        redraw()
    }

    /// The form is filled in: what it comes to goes into the field.
    public func submitForm() {
        form?.submit()
        endForm()
    }

    /// Never mind: the key that opened the form goes back into the field.
    public func cancelForm() {
        form?.cancel()
        endForm()
    }

    /// Whether the text view has the keyboard: it is the first responder of
    /// the key window.
    public func setViewHasKeyboard(_ hasKeyboard: Bool) {
        viewHasKeyboard = hasKeyboard
        updateClaim()
    }

    /// Whether the form's boxes have the keyboard, on the same terms.
    public func setFormHasKeyboard(_ hasKeyboard: Bool) {
        formHasKeyboard = hasKeyboard
        updateClaim()
    }

    /// The field is going: the draft changed or the editor closed. The
    /// keyboard is given back and an open form is cancelled with it.
    public func close() {
        if form != nil {
            cancelForm()
        }
        viewHasKeyboard = false
        formHasKeyboard = false
        updateClaim()
    }

    private func endForm() {
        form = nil
        formHasKeyboard = false
        updateClaim()
        redraw()
    }

    private func updateClaim() {
        let wanted = viewHasKeyboard || formHasKeyboard
        guard wanted != claimed else { return }
        claimed = wanted
        claimKeyboard(wanted)
    }

    private func redraw() {
        field = trial.field()
    }

    /// A text view reports Return as a line feed, and the tap reports it as a
    /// carriage return. The engine is fed what the tap would feed it, so a
    /// delimiter here is a delimiter everywhere.
    private static func fromView(_ scalar: Unicode.Scalar) -> Unicode.Scalar {
        scalar == "\n" ? "\r" : scalar
    }
}

/// Where a form's plan goes: back into the field it came from.
private final class TrialRunner: ExpansionRunner {
    private let trial: DraftTrial

    init(trial: DraftTrial) {
        self.trial = trial
    }

    func run(_ action: SessionAction) {
        guard case .expand(_, let steps, let undoDeleteCount, _) = action else { return }
        trial.finish(steps: steps, undoDeleteCount: undoDeleteCount)
    }

    func cancel(_ session: ExpansionSession) {
        trial.cancel(session: session)
    }
}
