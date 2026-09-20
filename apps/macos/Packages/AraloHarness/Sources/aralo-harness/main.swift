import AraloHarness
import Foundation

// The harness is a command-line tool and has to talk. It writes through these
// two and nothing else, and it never writes text it read out of an app.
func say(_ text: String) {
    FileHandle.standardError.write(Data((text + "\n").utf8))
}

func output(_ text: String) {
    FileHandle.standardOutput.write(Data(text.utf8))
}

func write(_ data: Data, to path: String?) throws {
    guard let path else { return }
    try data.write(to: URL(fileURLWithPath: path))
}

let words = Array(CommandLine.arguments.dropFirst())
if words.isEmpty || words.contains("--help") || words.contains("-h") {
    output(Arguments.usage)
    exit(0)
}

do {
    let arguments = try Arguments(words)
    let harness = Harness(arguments, say: say)
    switch arguments.verb {
    case .list:
        output(try harness.list())
    case .matrix, .latency:
        let report = try arguments.verb == .matrix ? harness.matrix() : harness.latency()
        let markdown = report.markdown(cases: arguments.cases)
        output(markdown)
        try write(Data(markdown.utf8), to: arguments.markdown)
        try write(report.json(), to: arguments.json)
        if let require = arguments.require, report.passedApps < require {
            say("\(report.passedApps) apps pass; \(require) are required.")
            exit(1)
        }
        if arguments.gate, !report.latencyWithinBudget {
            say("p95 is over \(Int(Report.latencyBudgetMs)) ms.")
            exit(1)
        }
    }
} catch let problem as Arguments.Problem {
    say("aralo-harness: \(problem)\n")
    say(Arguments.usage)
    exit(2)
} catch {
    say("aralo-harness: \(error)")
    exit(2)
}
