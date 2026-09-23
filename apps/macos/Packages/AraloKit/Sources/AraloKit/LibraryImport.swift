import AraloBridge
import Foundation
import Observation

/// One import, from choosing the file to reading the report: what the source
/// would become, the choices that change that, and then what it did become.
/// It is the import sheet apart from its window, so a second shell draws the
/// window and nothing else.
///
/// Nothing is written until `run()`. Until then every change of option runs
/// the import again as a dry run, so what the sheet shows is what the import
/// would do, not a guess at it (ADR-0013).
@MainActor
@Observable
public final class LibraryImport: Identifiable {
    /// What the source is. `automatic` lets the core work it out from the
    /// file, which is right for every file the corpus holds.
    public enum Format: String, CaseIterable, Sendable {
        case automatic
        case textexpander
        case csv
        case json
        case yaml

        public var title: String {
            switch self {
            case .automatic: "Work It Out"
            case .textexpander: "TextExpander"
            case .csv: "CSV"
            case .json: "JSON"
            case .yaml: "YAML"
            }
        }
    }

    /// One section of the report: the snippets that came out one way.
    public struct Section: Identifiable, Sendable {
        public let outcome: ImportOutcome
        public let entries: [ImportedSnippet]
        public var id: String { title }

        public var title: String {
            switch outcome {
            case .needsEdit: "Need an Edit"
            case .skipped: "Not Imported"
            case .clean: "Imported As They Were"
            }
        }
    }

    public let source: URL
    public var format: Format = .automatic {
        didSet { if format != oldValue { preview() } }
    }
    public var macros: MacroHandling = .auto {
        didSet { if macros != oldValue { preview() } }
    }
    /// The group everything goes into, `Work/Email` style. Empty keeps the
    /// source's own groups at the top of the library.
    public var group: String = "" {
        didSet { if group != oldValue { preview() } }
    }

    /// The last report: a dry run's until `run()` works, the import's after.
    public private(set) var summary: ImportSummary?
    /// Why the last attempt failed, in the core's words. A dry run that fails
    /// leaves no report, since there is nothing it would do.
    public private(set) var failure: String?
    /// The import has run and the folder holds what the report lists.
    public private(set) var isDone = false

    @ObservationIgnored private let core: Core
    @ObservationIgnored private let imported: (ImportSummary, [String]) -> Void

    public nonisolated var id: String { source.path }

    init(core: Core, source: URL, imported: @escaping (ImportSummary, [String]) -> Void) {
        self.core = core
        self.source = source
        self.imported = imported
        preview()
    }

    /// The source's file name, as the Finder shows it.
    public var sourceName: String { source.lastPathComponent }

    /// `group` as folder names, with empty parts dropped, so "Work/" and
    /// "/Work" both mean the one group.
    public var groupPath: [String] {
        group.split(separator: "/")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
    }

    /// Whether `run()` has anything to do. An import that would write nothing
    /// is not offered.
    public var canImport: Bool {
        !isDone && (summary?.imported ?? 0) > 0
    }

    /// The report, sorted by what needs the user: the snippets to fix first,
    /// then the ones left out, then the rest. Empty sections are left off.
    public var sections: [Section] {
        guard let summary else { return [] }
        return [ImportOutcome.needsEdit, .skipped, .clean].compactMap { outcome in
            let entries = summary.entries.filter { $0.outcome == outcome }
            return entries.isEmpty ? nil : Section(outcome: outcome, entries: entries)
        }
    }

    /// The report in one sentence, in the tense it is true in.
    public var headline: String {
        guard let summary else { return "" }
        let total = Self.count(Int(summary.total), "snippet")
        let imported = Int(summary.imported)
        if isDone {
            return imported == Int(summary.total)
                ? "Imported all \(total) from \(sourceName)."
                : "Imported \(imported) of \(total) from \(sourceName)."
        }
        if imported == 0 {
            return "\(sourceName) has \(total), and none of them can be imported."
        }
        return imported == Int(summary.total)
            ? "All \(total) in \(sourceName) can be imported."
            : "\(imported) of the \(total) in \(sourceName) can be imported."
    }

    /// How much of it comes over untouched, which is the number the fidelity
    /// harness measures, and what the rest needs.
    public var detail: String {
        guard let summary else { return "" }
        var parts = ["\(Int((summary.fidelity * 100).rounded()))% come over as they were"]
        if summary.needsEdit > 0 {
            let they = summary.needsEdit == 1 ? "it expands" : "they expand"
            parts.append("\(Self.count(Int(summary.needsEdit), "snippet")) need an edit, though \(they)")
        }
        if summary.skipped > 0 {
            parts.append("\(Self.count(Int(summary.skipped), "snippet")) cannot come over")
        }
        return parts.joined(separator: "; ") + "."
    }

    /// Where the snippets go, in words.
    public var destination: String {
        groupPath.isEmpty
            ? "Into the library, in the groups the file gives them."
            : "Into \u{201C}\(groupPath.joined(separator: " \u{203A} "))\u{201D}, in the groups the file gives them."
    }

    /// Works out what the import would do, and writes nothing.
    public func preview() {
        guard !isDone else { return }
        do {
            summary = try core.import(source: source.path, options: settings(dryRun: true))
            failure = nil
        } catch {
            summary = nil
            failure = error.reason
        }
    }

    /// Imports for real. False, with `failure` set, when that did not work;
    /// the dry run's report stays on screen then.
    @discardableResult
    public func run() -> Bool {
        guard !isDone else { return true }
        do {
            let done = try core.import(source: source.path, options: settings(dryRun: false))
            summary = done
            failure = nil
            isDone = true
            imported(done, groupPath)
            return true
        } catch {
            failure = error.reason
            return false
        }
    }

    private func settings(dryRun: Bool) -> ImportSettings {
        ImportSettings(
            format: format == .automatic ? nil : format.rawValue,
            intoGroup: groupPath,
            macros: macros,
            dryRun: dryRun
        )
    }

    static func count(_ number: Int, _ noun: String) -> String {
        number == 1 ? "1 \(noun)" : "\(number) \(noun)s"
    }
}

extension MacroHandling {
    /// What each choice does to a body, for a picker.
    public var title: String {
        switch self {
        case .auto: "Convert Where They Are Plain"
        case .convert: "Convert Them All"
        case .literal: "Keep the Text As It Is"
        case .template: "Already Aralo Templates"
        }
    }

    public static var allCases: [MacroHandling] { [.auto, .convert, .literal, .template] }
}
