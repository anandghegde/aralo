import Foundation

public enum Percentile {
    /// Nearest rank: the smallest sample that at least `percent` of them do not exceed.
    public static func of(_ samples: [Double], _ percent: Double) -> Double? {
        guard !samples.isEmpty else { return nil }
        let sorted = samples.sorted()
        let rank = Int((percent / 100 * Double(sorted.count)).rounded(.up))
        return sorted[min(max(rank, 1), sorted.count) - 1]
    }
}

/// A finished run: what CI publishes and what a person pastes into an issue.
public struct Report: Codable, Equatable, Sendable {
    public static let latencyBudgetMs = 50.0

    /// "chosen", "type" or "paste": which table Aralo ran with.
    public var config: String
    public var apps: [AppResult]
    /// Extra latency samples, by insert method, from `aralo-harness latency`.
    public var extraLatencyMs: [String: [Double]]

    enum CodingKeys: String, CodingKey {
        case config, apps
        case extraLatencyMs = "extra_latency_ms"
    }

    public init(config: String, apps: [AppResult], extraLatencyMs: [String: [Double]] = [:]) {
        self.config = config
        self.apps = apps
        self.extraLatencyMs = extraLatencyMs
    }

    public var tested: [AppResult] { apps.filter(\.tested) }
    public var passedApps: Int { apps.filter(\.passed).count }

    /// Cases that ran, and how many of them failed: the measure for the PRD's
    /// 0.1% failure target.
    public var caseCounts: (ran: Int, failed: Int) {
        let ran = tested.flatMap(\.cases).filter(\.counts)
        return (ran.count, ran.filter { !$0.passed }.count)
    }

    public var latencyMs: [String: [Double]] {
        var samples = extraLatencyMs
        for result in tested.flatMap(\.cases) {
            guard let latency = result.latencyMs, let method = result.method else { continue }
            samples[method, default: []].append(latency)
        }
        return samples
    }

    /// The typed-to-inserted gate: p95 within budget for every method measured.
    public var latencyWithinBudget: Bool {
        latencyMs.values.allSatisfy { (Percentile.of($0, 95) ?? 0) <= Self.latencyBudgetMs }
    }

    public func json() throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return try encoder.encode(self)
    }

    // MARK: Markdown

    public func markdown(cases: [MatrixCase] = MatrixCase.allCases) -> String {
        var lines: [String] = []
        if !apps.isEmpty {
            lines += ["## Injection matrix (\(config) method)", ""]
            lines.append("| App | " + cases.map(\.rawValue).joined(separator: " | ") + " | Result |")
            lines.append("| --- |" + String(repeating: " --- |", count: cases.count + 1))
            lines += apps.map { row($0, cases) }
            lines.append("")
            let counts = caseCounts
            let rate = counts.ran == 0 ? 0 : Double(counts.failed) / Double(counts.ran) * 100
            lines.append("**\(passedApps) of \(apps.count) apps pass.** Tested: \(tested.count). "
                + "\(counts.failed) of \(counts.ran) cases failed (\(Self.number(rate))%).")
            lines += notes()
        }
        let latency = latencyTable()
        if !lines.isEmpty, !latency.isEmpty { lines.append("") }
        lines += latency
        return lines.joined(separator: "\n") + "\n"
    }

    private func row(_ app: AppResult, _ cases: [MatrixCase]) -> String {
        guard app.tested else {
            let blanks = String(repeating: " – |", count: cases.count)
            return "| \(app.name) |\(blanks) skipped: \(app.skipped ?? "") |"
        }
        let cells = cases.map { matrixCase -> String in
            guard let result = app.cases.first(where: { $0.matrixCase == matrixCase }) else { return "–" }
            switch result.status {
            case .pass: return "pass"
            case .flaky: return "pass (attempt \(result.attempts))"
            case .fail: return "**FAIL**"
            case .pending: return "pending"
            case .focusLost: return "focus lost"
            }
        }
        return "| \(app.name) | " + cells.joined(separator: " | ") + " | \(app.passed ? "pass" : "**FAIL**") |"
    }

    private func notes() -> [String] {
        let failures = tested.flatMap { app in
            app.cases.filter { $0.counts && !$0.passed }.map { result in
                "- \(app.name), \(result.matrixCase.rawValue): \(result.reason ?? result.status.rawValue)"
            }
        }
        return failures.isEmpty ? [] : ["", "Failures:", ""] + failures
    }

    private func latencyTable() -> [String] {
        let samples = latencyMs.filter { !$0.value.isEmpty }
        guard !samples.isEmpty else { return [] }
        var lines = [
            "## Typed to inserted", "",
            "| Method | Samples | p50 ms | p95 ms | p99 ms | max ms | p95 within \(Int(Self.latencyBudgetMs)) ms |",
            "| --- | --- | --- | --- | --- | --- | --- |"
        ]
        for (method, values) in samples.sorted(by: { $0.key < $1.key }) {
            let cells = [50.0, 95, 99, 100].map { Self.number(Percentile.of(values, $0) ?? 0) }
            let within = (Percentile.of(values, 95) ?? 0) <= Self.latencyBudgetMs
            lines.append("| \(method) | \(values.count) | " + cells.joined(separator: " | ")
                + " | \(within ? "yes" : "**no**") |")
        }
        lines += ["", "Each figure includes the harness's own Accessibility read, so it is an upper bound."]
        return lines
    }

    private static func number(_ value: Double) -> String {
        String(format: "%.1f", value)
    }
}
