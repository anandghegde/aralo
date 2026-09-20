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

    /// Returns true when the event must not reach the app.
    public func handle(_ event: TapEvent) -> Bool {
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

    private func act(on action: KeyAction) -> Bool {
        switch action {
        case .pass:
            return false
        case .expand(let snippetId, let consume, let steps, let undoDeleteCount, let profile):
            schedule { [engine, injector] in
                let outcome = injector.run(steps, profile: profile)
                if let undoDeleteCount, outcome.undoable {
                    engine.expansionDone(snippetId: snippetId, deleteCount: undoDeleteCount, method: outcome.method)
                }
            }
            return consume
        case .undoExpansion(let deleteCount, let retype, let method, let profile):
            schedule { [injector] in
                injector.undo(deleteCount: deleteCount, retype: retype, method: method, profile: profile)
            }
            return true
        }
    }
}
