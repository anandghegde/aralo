import AraloBridge
import Foundation

/// Runs the cases in one app. It knows nothing about the operating system; it
/// sees the world through a `Desktop`.
@MainActor
public final class MatrixRunner {
    public struct Options: Sendable {
        /// A failure counts only when it happens this many times in a row.
        public var attempts = 3
        /// How long an expansion gets to appear, before `MatrixCase.patience`.
        public var timeout: TimeInterval = 5
        public var pollInterval: TimeInterval = 0.002
        /// Between an expansion and the key that follows it.
        public var pause: TimeInterval = 0.15
        public var includePending = false

        public init() {}
    }

    /// The keyboard went to another app. Nothing more may be typed.
    struct FocusLost: Error {}

    struct Attempt {
        var passed: Bool
        var reason: String?
        var readBack: ReadBack?
        var latencyMs: Double?
    }

    private let desktop: Desktop
    private let options: Options

    public init(desktop: Desktop, options: Options = Options()) {
        self.desktop = desktop
        self.options = options
    }

    public func run(
        _ app: CompatApp, recipe: Recipe, cases: [MatrixCase], expectations: [MatrixCase: Expectation]
    ) -> AppResult {
        var result = AppResult(name: app.name, bundleID: app.bundleId)
        if let why = open(app, recipe: recipe) {
            result.skipped = why
            return result
        }
        defer { desktop.dismiss(app) }

        for matrixCase in cases {
            let outcome = run(matrixCase, in: app, recipe: recipe, expectation: expectations[matrixCase])
            result.cases.append(outcome)
            if outcome.status == .focusLost { break }
        }
        return result
    }

    /// Brings the app up. Returns why it cannot be tested, or nil when its
    /// field is ready.
    private func open(_ app: CompatApp, recipe: Recipe) -> String? {
        switch desktop.bringUp(app, recipe: recipe) {
        case .ready: break
        case .notInstalled: return "not installed"
        case .unreachable(let why): return why
        }
        // Clearing selects everything and deletes it, so the harness clears
        // only a field it found empty: text it did not type is not its to delete.
        if recipe.clearing == .selectAll, desktop.focusedText()?.isEmpty == false {
            desktop.dismiss(app)
            return "the field with the keyboard already has text in it"
        }
        return nil
    }

    public struct LatencyRun: Equatable, Sendable {
        /// Milliseconds from the delimiter to the text, by insert method.
        public var samples: [String: [Double]] = [:]
        public var failures = 0
        public var skipped: String?
    }

    /// The same cases over and over in one app, for the latency figures.
    public func measure(
        _ app: CompatApp, recipe: Recipe, cases: [MatrixCase], runs: Int, expectations: [MatrixCase: Expectation]
    ) -> LatencyRun {
        var run = LatencyRun()
        if let why = open(app, recipe: recipe) {
            run.skipped = why
            return run
        }
        defer { desktop.dismiss(app) }

        for matrixCase in cases.filter(\.measuresLatency) {
            for _ in 0..<runs {
                let result = self.run(matrixCase, in: app, recipe: recipe, expectation: expectations[matrixCase])
                if result.status == .focusLost {
                    run.skipped = "another app took the keyboard"
                    return run
                }
                if let latency = result.latencyMs, let method = result.method {
                    run.samples[method, default: []].append(latency)
                } else {
                    run.failures += 1
                }
            }
        }
        return run
    }

    func run(_ matrixCase: MatrixCase, in app: CompatApp, recipe: Recipe, expectation: Expectation?) -> CaseResult {
        if let reason = matrixCase.pendingReason, !options.includePending {
            return CaseResult(matrixCase, .pending, reason: reason)
        }
        guard let expectation else {
            return CaseResult(matrixCase, .fail, reason: "the core gave no expansion to compare with")
        }
        let method = expectation.method == .pasted ? "paste" : "type"
        var last = Attempt(passed: false)
        for attempt in 1...max(1, options.attempts) {
            do {
                last = try tryOnce(matrixCase, in: app, recipe: recipe, expectation: expectation)
                // Also the last look at who has the keyboard: what was read
                // back counts only if it was read from the app under test.
                try clear(app, recipe)
            } catch {
                return CaseResult(
                    matrixCase, .focusLost, attempts: attempt, method: method,
                    reason: "another app took the keyboard"
                )
            }
            guard last.passed else { continue }
            return CaseResult(
                matrixCase, attempt == 1 ? .pass : .flaky, attempts: attempt, method: method,
                readBack: last.readBack, latencyMs: attempt == 1 ? last.latencyMs : nil
            )
        }
        return CaseResult(
            matrixCase, .fail, attempts: max(1, options.attempts), method: method,
            readBack: last.readBack, reason: last.reason
        )
    }

    // MARK: One attempt

    private func tryOnce(
        _ matrixCase: MatrixCase, in app: CompatApp, recipe: Recipe, expectation: Expectation
    ) throws -> Attempt {
        try clear(app, recipe)
        if desktop.focusedText()?.contains(expectation.text) == true {
            return Attempt(passed: false, reason: "the field could not be emptied")
        }
        let sentinel = "aralo-matrix-\(UUID().uuidString)"
        if matrixCase == .clipboard { desktop.clipboard = sentinel }

        try checkFront(app)
        guard desktop.type(matrixCase.abbreviation) else {
            return Attempt(passed: false, reason: "the keyboard layout cannot type the abbreviation")
        }
        try checkFront(app)
        let started = desktop.now()
        _ = desktop.type(Expectation.delimiter)

        let patience = options.timeout * matrixCase.patience
        let seenAfter = waitForText(patience) { $0.contains(expectation.text) }
        var attempt = Attempt(passed: seenAfter != nil, readBack: seenAfter == nil ? nil : .accessibility)
        if let seenAfter, matrixCase.measuresLatency {
            attempt.latencyMs = (seenAfter - started) * 1000
        }

        if matrixCase == .clipboard {
            // Aralo puts the clipboard back once the app has read the paste.
            desktop.wait(TimeInterval(app.profile.pasteSettleMs) / 1000 + 0.5)
            guard desktop.clipboard == sentinel else {
                return Attempt(passed: false, reason: "the clipboard was not put back", readBack: attempt.readBack)
            }
        }
        if !attempt.passed {
            attempt = try confirmByCopy(app, expectation: expectation)
        }
        guard attempt.passed else { return attempt }
        return try followUp(matrixCase, in: app, expectation: expectation, seen: attempt)
    }

    /// The second half of the cases that press something after the expansion.
    private func followUp(
        _ matrixCase: MatrixCase, in app: CompatApp, expectation: Expectation, seen: Attempt
    ) throws -> Attempt {
        var attempt = seen
        switch matrixCase {
        case .undo:
            desktop.wait(options.pause)
            try checkFront(app)
            desktop.press(.undo)
            let undone: (String) -> Bool = { $0.contains(expectation.typed) && !$0.contains(expectation.text) }
            attempt.passed = try settles(app, within: options.timeout, undone)
            attempt.reason = attempt.passed ? nil : "undo did not bring the abbreviation back"
        case .cursor:
            desktop.wait(options.pause)
            try checkFront(app)
            _ = desktop.type("x")
            let wanted = expectation.text(withAtCaret: "x")
            attempt.passed = try settles(app, within: options.timeout) { $0.contains(wanted) }
            attempt.reason = attempt.passed ? nil : "the caret was not where the marker is"
        case .ascii, .unicode, .long, .clipboard:
            break
        }
        return attempt
    }

    // MARK: Reading the field

    /// Polls the Accessibility value. Returns the clock when `matches` first
    /// held. An app that tells Accessibility nothing gets the whole timeout,
    /// because nothing else says when its expansion has finished.
    private func waitForText(_ timeout: TimeInterval, _ matches: (String) -> Bool) -> TimeInterval? {
        let started = desktop.now()
        while desktop.now() - started < timeout {
            if let text = desktop.focusedText(), matches(text) { return desktop.now() }
            desktop.wait(options.pollInterval)
        }
        return nil
    }

    private func settles(_ app: CompatApp, within timeout: TimeInterval, _ matches: (String) -> Bool) throws -> Bool {
        if waitForText(timeout, matches) != nil { return true }
        return try copied(app).map(matches) ?? false
    }

    /// For apps that tell Accessibility nothing: select all, copy, read the
    /// clipboard, and give the clipboard back.
    private func confirmByCopy(_ app: CompatApp, expectation: Expectation) throws -> Attempt {
        guard let text = try copied(app) else {
            return Attempt(passed: false, reason: "the field could not be read, by Accessibility or by copying")
        }
        if text.contains(expectation.text) { return Attempt(passed: true, readBack: .copy) }
        let leftTyped = text.contains(expectation.typed.trimmingCharacters(in: .whitespaces))
        return Attempt(
            passed: false,
            reason: leftTyped ? "the abbreviation was left as typed" : "the text that arrived is not the expansion",
            readBack: .copy
        )
    }

    private func copied(_ app: CompatApp) throws -> String? {
        let kept = desktop.clipboard
        defer { desktop.clipboard = kept }
        let marker = "aralo-matrix-copy-\(UUID().uuidString)"
        desktop.clipboard = marker
        for chord in [KeyChord.selectAll, .copy] {
            try checkFront(app)
            desktop.press(chord)
        }
        desktop.wait(options.pause)
        let text = desktop.clipboard
        return text == marker ? nil : text
    }

    private func clear(_ app: CompatApp, _ recipe: Recipe) throws {
        for chord in recipe.clearChords {
            try checkFront(app)
            desktop.press(chord)
        }
        desktop.wait(options.pause)
    }

    /// No key is sent unless the app under test has the keyboard. Select-all
    /// and Delete in somebody's open document would be unforgivable.
    private func checkFront(_ app: CompatApp) throws {
        guard desktop.frontmostBundleID()?.caseInsensitiveCompare(app.bundleId) == .orderedSame else {
            throw FocusLost()
        }
    }
}
