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
    private let schedule: Schedule

    /// `injector` is only ever used inside `schedule`, which must run its
    /// blocks one at a time.
    public init(engine: EngineProtocol, injector: Injector, schedule: @escaping Schedule) {
        self.engine = engine
        self.injector = injector
        self.schedule = schedule
    }

    public convenience init(engine: EngineProtocol, injector: Injector) {
        let queue = DispatchQueue(label: "app.aralo.injector", qos: .userInteractive)
        self.init(engine: engine, injector: injector) { queue.async(execute: $0) }
    }

    /// Returns true when the event must not reach the app.
    public func handle(_ event: TapEvent) -> Bool {
        guard case .key(let stroke) = event else {
            engine.reset(reason: .mouseDown)
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
        case .expand(let snippetId, let consume, let steps, let undoDeleteCount):
            schedule { [engine, injector] in
                let method = injector.run(steps)
                if let undoDeleteCount {
                    engine.expansionDone(snippetId: snippetId, deleteCount: undoDeleteCount, method: method)
                }
            }
            return consume
        case .undoExpansion(let deleteCount, let retype, let method):
            schedule { [injector] in
                injector.undo(deleteCount: deleteCount, retype: retype, method: method)
            }
            return true
        }
    }
}
