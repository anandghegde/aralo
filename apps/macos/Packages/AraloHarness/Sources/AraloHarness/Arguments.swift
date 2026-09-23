import Foundation

/// The command line, parsed. No dependency for this: three verbs and a dozen flags.
public struct Arguments: Equatable, Sendable {
    public enum Verb: String, Sendable {
        case list
        case matrix
        case latency
    }

    public var verb: Verb
    public var config = CompatConfig.chosen
    public var undo = "native"
    /// Bundle IDs to run, in place of the whole table.
    public var only: [String] = []
    public var cases = MatrixCase.allCases
    public var attempts = 3
    /// Seconds a form panel gets to open before its answer is typed.
    public var formPause = 0.75
    public var includePending = false
    /// Seconds a person gets to click into a field. Zero: nobody is there.
    public var manualWait = 0.0
    public var runs = 50
    /// `matrix`: at least this many apps must pass. `latency`: p95 must be in budget.
    public var require: Int?
    public var gate = false
    public var launchAralo = true
    public var aralo = "target/xcode/Build/Products/Debug/Aralo.app"
    public var library = "fixtures/matrix/library"
    public var recipes = "data/compat/matrix.json"
    public var json: String?
    public var markdown: String?

    public struct Problem: Error, Equatable, CustomStringConvertible {
        public var description: String
    }

    public static let usage = """
        aralo-harness: type into real apps while Aralo runs, and report what arrived.

          aralo-harness list                 the apps, their recipes and methods; touches nothing
          aralo-harness matrix  [options]    every case in every app of the compatibility table
          aralo-harness latency [options]    typed-to-inserted time, many runs in one app

        Options:
          --method chosen|type|paste   the table as shipped, or one method forced on every app [chosen]
          --undo native|backspace      undo style for a forced method [native]
          --only ID[,ID...]            just these bundle IDs (latency: the first; default TextEdit)
          --cases NAME[,NAME...]       of: ascii unicode long cursor undo clipboard form
          --attempts N                 a failure counts after N in a row [3]
          --form-pause SECONDS         how long the form panel gets to open, for a slow machine [0.75]
          --include-pending            also run cases that wait on a later milestone
          --manual-wait SECONDS        let a person click into apps that need it [0: skip them]
          --runs N                     latency: expansions per insert method [50]
          --require N                  matrix: exit 1 unless N apps pass
          --gate                       latency: exit 1 unless p95 is within 50 ms
          --no-launch                  use the Aralo that is running, as it is
          --aralo PATH  --library PATH  --recipes PATH
          --json PATH   --markdown PATH

        Run it from the repository root, in a terminal that has Accessibility access. It takes
        the keyboard: keep your hands off until it is done. Moving to another app stops it.

        """

    public init(verb: Verb) {
        self.verb = verb
    }

    public init(_ words: [String]) throws {
        guard let first = words.first, let verb = Verb(rawValue: first) else {
            throw Problem(description: words.isEmpty ? "say what to do" : "\"\(words[0])\" is not a command")
        }
        self.init(verb: verb)
        var rest = words.dropFirst()[...]
        while let flag = rest.popFirst() {
            try read(flag, &rest)
        }
    }

    private mutating func read(_ flag: String, _ rest: inout ArraySlice<String>) throws {
        switch flag {
        case "--include-pending": includePending = true
        case "--gate": gate = true
        case "--no-launch": launchAralo = false
        default:
            guard Self.valued.contains(flag) else { throw Problem(description: "\(flag): no such option") }
            guard let value = rest.popFirst() else { throw Problem(description: "\(flag) needs a value") }
            try setChoice(flag, value)
            try setCount(flag, value)
            try setSeconds(flag, value)
            setPath(flag, value)
        }
    }

    private static let valued: Set<String> = [
        "--method", "--undo", "--only", "--cases", "--attempts", "--manual-wait", "--runs", "--require",
        "--form-pause", "--aralo", "--library", "--recipes", "--json", "--markdown"
    ]

    private mutating func setChoice(_ flag: String, _ text: String) throws {
        let parts = text.split(separator: ",").map(String.init)
        switch flag {
        case "--method": config = try Self.pick(CompatConfig.self, text, flag)
        case "--undo": undo = try Self.oneOf(["native", "backspace"], text, flag)
        case "--only": only = parts
        case "--cases": cases = try parts.map { try Self.pick(MatrixCase.self, $0, flag) }
        default: break
        }
    }

    private mutating func setCount(_ flag: String, _ text: String) throws {
        guard ["--attempts", "--manual-wait", "--runs", "--require"].contains(flag) else { return }
        guard let number = Int(text), number >= 0 else { throw Problem(description: "\(flag) \(text): not a count") }
        switch flag {
        case "--attempts": attempts = max(1, number)
        case "--manual-wait": manualWait = Double(number)
        case "--runs": runs = max(1, number)
        default: require = number
        }
    }

    /// The one flag measured in fractions of a second: a panel that opens in a
    /// third of a second is worth saying so.
    private mutating func setSeconds(_ flag: String, _ text: String) throws {
        guard flag == "--form-pause" else { return }
        guard let seconds = Double(text), seconds >= 0, seconds.isFinite else {
            throw Problem(description: "\(flag) \(text): not a number of seconds")
        }
        formPause = seconds
    }

    private mutating func setPath(_ flag: String, _ text: String) {
        switch flag {
        case "--aralo": aralo = text
        case "--library": library = text
        case "--recipes": recipes = text
        case "--json": json = text
        case "--markdown": markdown = text
        default: break
        }
    }

    private static func pick<Choice: RawRepresentable>(
        _: Choice.Type, _ text: String, _ flag: String
    ) throws -> Choice where Choice.RawValue == String {
        guard let choice = Choice(rawValue: text) else { throw Problem(description: "\(flag) \(text): no such choice") }
        return choice
    }

    private static func oneOf(_ choices: [String], _ text: String, _ flag: String) throws -> String {
        guard choices.contains(text) else { throw Problem(description: "\(flag) \(text): no such choice") }
        return text
    }
}
