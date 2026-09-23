import AraloBridge
import Foundation

/// What the tap reports.
public enum TapEvent: Equatable, Sendable {
    case key(KeyStroke)
    case mouseDown
}

/// The decision for every event the tap sees: ask the engine, and when it
/// matches, hand the plan to the injector. Runs on the tap thread, so it never
/// waits: injection happens on `schedule`'s queue.
public final class ExpansionController: @unchecked Sendable {
    public typealias Schedule = (@escaping @Sendable () -> Void) -> Void

    private let engine: EngineProtocol
    private let injector: Injector
    private let translator: KeyTranslator
    private let schedule: Schedule

    /// Guards `isSuspended` and `sessionHandler`, which the shell sets on the
    /// main thread and the tap reads on its own.
    private let lock = NSLock()
    private var isSuspended = false
    private var sessionHandler: (@Sendable (ExpansionSession) -> Void)?

    /// `injector` is only ever used inside `schedule`, which must run its
    /// blocks one at a time.
    public init(
        engine: EngineProtocol,
        injector: Injector,
        translator: KeyTranslator = KeyTranslator(),
        schedule: @escaping Schedule
    ) {
        self.engine = engine
        self.injector = injector
        self.translator = translator
        self.schedule = schedule
    }

    public convenience init(engine: EngineProtocol, injector: Injector, translator: KeyTranslator = KeyTranslator()) {
        let queue = DispatchQueue(label: "app.aralo.injector", qos: .userInteractive)
        self.init(engine: engine, injector: injector, translator: translator) { queue.async(execute: $0) }
    }

    /// Stops watching what is typed, for as long as a window of Aralo's own has
    /// the keyboard.
    ///
    /// What a user types into the search palette is a query, not text in a
    /// document: it must not fill the matcher's buffer, and an abbreviation
    /// typed there must not expand into a field that is about to close. The
    /// buffer is emptied either way, so what was half-typed before the palette
    /// opened cannot join what is typed after it closes.
    public func setSuspended(_ suspended: Bool) {
        lock.lock()
        let changed = isSuspended != suspended
        isSuspended = suspended
        lock.unlock()
        guard changed else { return }
        translator.clear()
        engine.reset(reason: .focusChange)
    }

    /// Sets who puts a snippet's questions in front of the user.
    ///
    /// The handler is called on the tap thread, like everything else here, and
    /// hops to the main actor itself. Until one is set there is nobody to ask,
    /// so a session is cancelled as soon as it starts and whatever it swallowed
    /// goes back: a half-open expansion is worse than none.
    public func setSessionHandler(_ handler: @escaping @Sendable (ExpansionSession) -> Void) {
        lock.lock()
        sessionHandler = handler
        lock.unlock()
    }

    /// Returns true when the event must not reach the app.
    public func handle(_ event: TapEvent) -> Bool {
        lock.lock()
        let suspended = isSuspended
        lock.unlock()
        // Everything reaches the app, which is Aralo's own window: the palette
        // needs its keystrokes, and nothing is decided about them here.
        if suspended {
            return false
        }
        guard case .key(var stroke) = event else {
            translator.clear()
            engine.reset(reason: .mouseDown)
            return false
        }
        // An input method is composing: what the document gets is not what
        // the keys say. The engine was reset when the input source changed.
        if translator.isComposing {
            return false
        }
        if KeyClassifier.typesText(stroke) {
            stroke.text = translator.text(keyCode: stroke.keyCode, flags: stroke.flags) ?? stroke.text
        } else if translator.clear() {
            // The key cancelled a dead key instead of doing its usual job:
            // Backspace removed the waiting accent, not a character.
            engine.reset(reason: .inputMethod)
            return false
        }
        switch KeyClassifier.classify(stroke) {
        case .ignore:
            return false
        case .reset(let reason):
            engine.reset(reason: reason)
            return false
        case .input(let key):
            return act(on: engine.onKey(key: key))
        }
    }

    /// Inserts a snippet the user picked from a list into `app`, which must be
    /// the app that has the keyboard by the time this is called.
    ///
    /// Nil when the text is on its way in; otherwise why nothing is. The
    /// insertion runs on the same queue as a typed expansion and is undone by
    /// the same key, because to the user it is the same thing arriving by
    /// another route.
    @discardableResult
    public func insert(snippetId: String, into app: String) -> InsertRefusal? {
        switch engine.insert(snippetId: snippetId, intoApp: app) {
        case .refused(let reason):
            return reason
        case .insert(let snippetId, let steps, let undoDeleteCount, let profile):
            // The palette took the keyboard while it was open, and whatever was
            // typed into it is not text in the app's document.
            translator.clear()
            run(snippetId: snippetId, steps: steps, undoDeleteCount: undoDeleteCount, profile: profile)
            return nil
        case .startSession(_, let session):
            translator.clear()
            // Nothing is refused and nothing is typed yet: the snippet has
            // something to ask, and the panel asks it.
            start(session)
            return nil
        }
    }

    private func act(on action: KeyAction) -> Bool {
        switch action {
        case .pass:
            return false
        case .expand(let snippetId, let consume, let steps, let undoDeleteCount, let profile):
            run(snippetId: snippetId, steps: steps, undoDeleteCount: undoDeleteCount, profile: profile)
            return consume
        case .startSession(_, let consume, let session):
            // The abbreviation matched, but the body asks something first.
            // Nothing reaches the document until it has been answered, and the
            // key that triggered the match is swallowed until then.
            start(session)
            return consume
        case .undoExpansion(let deleteCount, let retype, let method, let profile):
            schedule { [injector] in
                injector.undo(deleteCount: deleteCount, retype: retype, method: method, profile: profile)
            }
            return true
        }
    }

    /// Hands a session to whoever drives it, or ends it when nobody does.
    private func start(_ session: ExpansionSession) {
        lock.lock()
        let handler = sessionHandler
        lock.unlock()
        guard let handler else {
            cancel(session)
            return
        }
        handler(session)
    }

    /// Types a plan and tells the engine it landed, so the undo key knows what
    /// to take back. The report goes in only when the injector says the text
    /// arrived in a way it can undo, and only from the queue, after the last
    /// step: undo must never be armed for text that is still being typed.
    private func run(snippetId: String, steps: [PlanStep], undoDeleteCount: UInt32?, profile: InjectionProfile) {
        schedule { [engine, injector] in
            let outcome = injector.run(steps, profile: profile)
            if let undoDeleteCount, outcome.undoable {
                engine.expansionDone(snippetId: snippetId, deleteCount: undoDeleteCount, method: outcome.method)
            }
        }
    }
}

/// The panel's end of a session: whatever it decided is typed here, on the
/// queue and by the injector every other expansion uses, because to the user
/// it is the same thing arriving by another route.
extension ExpansionController: ExpansionRunner {
    public func run(_ action: SessionAction) {
        guard case .expand(let snippetId, let steps, let undoDeleteCount, let profile) = action else { return }
        run(snippetId: snippetId, steps: steps, undoDeleteCount: undoDeleteCount, profile: profile)
    }

    public func cancel(_ session: ExpansionSession) {
        let steps = session.cancel()
        guard !steps.isEmpty else { return }
        // Nothing was inserted, so there is nothing to undo and nothing to
        // report: the profile is only how to type, and what is being typed is
        // the character the match swallowed.
        let profile = engine.injectionProfile()
        schedule { [injector] in
            _ = injector.run(steps, profile: profile)
        }
    }
}
