import AraloBridge
import AraloKit
import Foundation

/// The three commands, between the command line and the runner.
@MainActor
public struct Harness {
    public struct Problem: Error, CustomStringConvertible {
        public var description: String
    }

    /// One app, with the row of the table Aralo will actually use for it.
    public struct Target {
        public var app: CompatApp
        public var recipe: Recipe
        public var expectations: [MatrixCase: Expectation]
    }

    public let arguments: Arguments
    private let say: (String) -> Void

    public init(_ arguments: Arguments, say: @escaping (String) -> Void) {
        self.arguments = arguments
        self.say = say
    }

    // MARK: What to run

    /// The apps of the shipped table, each with the profile and the expected
    /// text under `compatTable` (the forced-method table, or nil for the shipped one).
    public func targets(compatTable: URL?) throws -> [Target] {
        let book = try RecipeBook.load(URL(fileURLWithPath: arguments.recipes))
        let core = try Core.openLibrary(path: arguments.library)
        if let compatTable { try core.loadCompatTable(path: compatTable.path) }
        let engine = core.engine()

        var apps = try compatApps(path: nil)
        if !arguments.only.isEmpty {
            let unknown = arguments.only.filter { wanted in
                !apps.contains { $0.bundleId.caseInsensitiveCompare(wanted) == .orderedSame }
            }
            guard unknown.isEmpty else {
                throw Problem(description: "not in data/compat/apps.toml: \(unknown.joined(separator: ", "))")
            }
            apps = arguments.only.compactMap { wanted in
                apps.first { $0.bundleId.caseInsensitiveCompare(wanted) == .orderedSame }
            }
        }
        return try apps.map { app in
            engine.setFrontApp(bundleId: app.bundleId)
            let resolved = CompatApp(name: app.name, bundleId: app.bundleId, profile: engine.injectionProfile())
            var expectations: [MatrixCase: Expectation] = [:]
            for matrixCase in MatrixCase.allCases {
                expectations[matrixCase] = try Expectation.ask(engine, abbreviation: matrixCase.abbreviation)
            }
            return Target(app: resolved, recipe: book.recipe(for: app.bundleId), expectations: expectations)
        }
    }

    public func list() throws -> String {
        var lines = [
            "| App | Bundle ID | Opens by | Clears by | Insert | Undo |",
            "| --- | --- | --- | --- | --- | --- |"
        ]
        for target in try targets(compatTable: nil) {
            let profile = target.app.profile
            lines.append("| \(target.app.name) | \(target.app.bundleId) | \(target.recipe.open.rawValue) | "
                + "\(target.recipe.clearing.rawValue) | \(profile.insert) | \(profile.undo) |")
        }
        return lines.joined(separator: "\n") + "\n"
    }

    // MARK: Running

    /// Runs `body` with a desktop, an Aralo on the matrix library, and the
    /// person's clipboard put back afterwards.
    private func withAralo<Result>(_ body: (SystemDesktop, [Target]) throws -> Result) throws -> Result {
        guard Permissions.status.accessibility else {
            throw Problem(description: """
                this terminal has no Accessibility access, so it can neither press keys nor read fields. \
                Allow it under System Settings › Privacy & Security › Accessibility, then run this again.
                """)
        }
        let desktop = try SystemDesktop()
        desktop.manualWait = arguments.manualWait
        desktop.say = say
        let kept = desktop.clipboard
        defer {
            desktop.clipboard = kept
            desktop.cleanUp()
        }

        var compatTable: URL?
        if let table = arguments.config.table(undo: arguments.undo) {
            let file = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-matrix-apps.toml")
            try Data(table.utf8).write(to: file)
            compatTable = file
        }
        let targets = try targets(compatTable: compatTable)
        // Registered before the launch, so an Aralo that came up without its
        // tap is not left running either.
        defer { if arguments.launchAralo { AraloApp.stop(desktop: desktop) } }
        if arguments.launchAralo {
            let library = URL(fileURLWithPath: arguments.library).standardizedFileURL
            let app = URL(fileURLWithPath: arguments.aralo).standardizedFileURL
            try AraloApp.restart(app, library: library, compatTable: compatTable, desktop: desktop)
        } else if !AraloApp.isRunning {
            throw Problem(description: "Aralo is not running, and --no-launch says not to start it")
        } else {
            try AraloApp.waitForTap(desktop: desktop, timeout: 0)
        }
        return try body(desktop, targets)
    }

    private var options: MatrixRunner.Options {
        var options = MatrixRunner.Options()
        options.attempts = arguments.attempts
        options.includePending = arguments.includePending
        return options
    }

    public func matrix() throws -> Report {
        try withAralo { desktop, targets in
            let runner = MatrixRunner(desktop: desktop, options: options)
            var results: [AppResult] = []
            var focusLosses = 0
            for target in targets {
                say("\(target.app.name)…")
                let result = runner.run(
                    target.app, recipe: target.recipe, cases: arguments.cases, expectations: target.expectations
                )
                results.append(result)
                focusLosses += result.cases.filter { $0.status == .focusLost }.count
                // Somebody is using the machine. Stop taking the keyboard from them.
                if focusLosses >= 3 {
                    say("Another app took the keyboard three times. Stopping.")
                    break
                }
            }
            return Report(config: arguments.config.rawValue, apps: results)
        }
    }

    public func latency() throws -> Report {
        try withAralo { desktop, targets in
            let wanted = arguments.only.first ?? "com.apple.TextEdit"
            guard let target = targets.first(where: { $0.app.bundleId.caseInsensitiveCompare(wanted) == .orderedSame })
            else { throw Problem(description: "\(wanted) is not in data/compat/apps.toml") }
            var options = options
            options.attempts = 1
            let runner = MatrixRunner(desktop: desktop, options: options)
            let run = runner.measure(
                target.app, recipe: target.recipe, cases: [.ascii, .long], runs: arguments.runs,
                expectations: target.expectations
            )
            if let skipped = run.skipped { throw Problem(description: "\(target.app.name): \(skipped)") }
            if run.failures > 0 { say("\(run.failures) expansions did not arrive and are not in the figures.") }
            return Report(config: arguments.config.rawValue, apps: [], extraLatencyMs: run.samples)
        }
    }
}
