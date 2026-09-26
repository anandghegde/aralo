import AppKit
@testable import Aralo
import AraloBridge
import AraloKit
import SwiftUI
import XCTest

/// The plan's accessibility check (section 8, task 5.9): every panel the app
/// shows, drawn with real content, has no element VoiceOver could only read
/// out by its role. The audit is `AccessibilityAudit`; this is the list of
/// panels it runs over.
@MainActor
final class PanelAccessibilityTests: XCTestCase {
    private var folder: URL!
    private var cache: URL!
    private var core: Core!

    override func setUpWithError() throws {
        let run = UUID().uuidString
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-a11y-\(run)")
        cache = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-a11y-\(run)-cache")
        // A new folder, so the core writes the starter snippets and the
        // panels have something in them to draw.
        core = try Core.openLibrary(path: folder.path, cache: cache.path, events: nil, trash: nil)
    }

    override func tearDownWithError() throws {
        core = nil
        try? FileManager.default.removeItem(at: folder)
        try? FileManager.default.removeItem(at: cache)
    }

    /// Draws a view in a window of the size its controller gives it and fails
    /// on anything unnamed, with the tree VoiceOver sees to find it in.
    private func assertLabelled(
        _ view: some View, width: CGFloat, height: CGFloat, _ panel: String,
        file: StaticString = #filePath, line: UInt = #line
    ) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: width, height: height), styleMask: [.titled],
            backing: .buffered, defer: false
        )
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: view.frame(width: width, height: height))
        defer { window.close() }
        let findings = AccessibilityAudit.unlabelled(in: window)
        guard !findings.isEmpty, let content = window.contentView else { return }
        let tree = AccessibilityAudit.outline(content).joined(separator: "\n")
        XCTFail("\(panel): \(findings)\n\(tree)", file: file, line: line)
    }

    private func profiles() throws -> AiProfiles {
        try AiProfiles.open(path: cache.appendingPathComponent("profiles.toml").path, keys: .memory)
    }

    func testEveryOnboardingScreen() {
        let service = AraloService(libraryURL: folder, cacheURL: cache)
        for step in OnboardingStep.allCases {
            let model = OnboardingModel(service: service, startingAt: step)
            assertLabelled(OnboardingView(model: model), width: 520, height: 420, "\(step)")
        }
    }

    func testTheLibraryWindowWithASnippetOpen() throws {
        let store = LibraryStore(core: core)
        let first = try XCTUnwrap(core.snippets().first)
        store.select(snippet: first.id)
        assertLabelled(LibraryView(store: store, root: folder), width: 960, height: 600, "library")
    }

    func testTheAISettingsWithAProfileBeingAdded() throws {
        let store = AISettingsStore(settings: try profiles())
        assertLabelled(AISettingsView(store: store), width: 760, height: 560, "AI settings")
        store.newProfile()
        assertLabelled(AISettingsView(store: store), width: 760, height: 560, "new profile")
    }

    func testThePalette() {
        let store = PaletteStore(core: core, inserter: NoInserter())
        assertLabelled(
            PaletteView(store: store, focus: PaletteFocus(), insert: {}, cancel: {}), width: 640, height: 400,
            "palette"
        )
    }

    func testTheCommandPanel() throws {
        let command = AiCommand(
            id: "01K5C0MMANDS00000000000001", label: "Fix spelling and grammar",
            instruction: "Fix it.", tags: [], profile: nil, model: nil, builtin: true
        )
        let store = CommandStore(
            commands: [command], selection: .success("their going home"), runner: try profiles(),
            access: NoSelection()
        )
        assertLabelled(CommandView(store: store, replace: {}, escape: {}), width: 640, height: 440, "commands")
    }

    func testTheEditorsAISheet() throws {
        let store = AuthoringStore(
            action: .proofread, replacing: "their going", range: NSRange(location: 0, length: 11),
            runner: try profiles()
        )
        assertLabelled(
            AuthoringSheet(store: store, replace: { _ in true }, close: {}), width: 560, height: 420, "AI sheet"
        )
    }

    // MARK: The form panel

    /// A session for `body`, picked from the palette as the form panel gets one.
    private func form(_ body: String, models: BlockRunner? = nil) throws -> FormSession {
        let id = try core.createSnippet(
            group: [],
            draft: SnippetDraft(
                label: "Ticket", abbreviations: [";tkt"], body: body, tags: [], kind: .text,
                trigger: nil, case: nil, wholeWord: nil, keepDelimiter: nil, enabled: nil
            )
        )
        try core.reload()
        guard case .startSession(_, let session) = core.engine().insert(snippetId: id, intoApp: "com.apple.TextEdit")
        else {
            struct NothingAsked: Error {}
            throw NothingAsked()
        }
        return FormSession(session: session, runner: NoRunner(), label: "Ticket", models: models)
    }

    private func assertLabelled(_ session: FormSession, _ panel: String, line: UInt = #line) {
        assertLabelled(
            FormView(session: session, submit: {}, cancel: {}), width: 460, height: 340, panel, line: line
        )
    }

    /// Waits for the models to stop writing, as the panel does before Enter.
    private func settled(_ session: FormSession) async {
        let deadline = ContinuousClock.now + .seconds(2)
        while session.isWriting, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(5))
        }
        XCTAssertFalse(session.isWriting, "the model never finished")
    }

    func testTheFormPanelWithEveryKindOfBox() throws {
        let session = try form(
            "Dear {{field: who | label: Their name | default: friend}},\n"
                + "{{field: notes | label: Notes | lines: 3}}\n"
                + "Sent by {{choice: how | options: post, e-mail}}."
        )
        XCTAssertEqual(session.fields.count, 3)
        assertLabelled(session, "form")
    }

    func testTheFormPanelOnItsAIBlocks() async throws {
        let session = try form(
            "Hi {{field: who}},\n{{ai: Thank them | fallback: Thanks.}}", models: CannedModel(answer: "Thank you!")
        )
        session.answers["who"] = "Dana"
        session.submit()
        XCTAssertEqual(session.step, .blocks)
        await settled(session)
        assertLabelled(session, "AI blocks, written")
        session.edit()
        assertLabelled(session, "AI blocks, editing")
    }

    func testTheFormPanelWhileAModelIsWriting() throws {
        let session = try form("{{ai: Thank them | fallback: Thanks.}}", models: CannedModel(answer: nil))
        XCTAssertTrue(session.isWriting)
        assertLabelled(session, "AI blocks, writing")
        session.cancel()
    }

    func testTheFormPanelWhenNoModelAnswers() async throws {
        let session = try form("{{ai: Thank them | fallback: Thanks.}}")
        await settled(session)
        assertLabelled(session, "AI blocks, fallback")
    }

    // MARK: The import sheet

    /// A CSV with one snippet of each outcome: one that comes over clean, one
    /// whose macro does not convert, and an empty row that cannot come over.
    private func export() throws -> URL {
        let file = cache.appendingPathComponent("export.csv")
        try FileManager.default.createDirectory(at: cache, withIntermediateDirectories: true)
        try """
            Abbreviation,Name,Folder,Content
            ;wait,Wait,Work,"Hold %delay:500% on"
            ;sig,Signature,Work,"Best, Dana"
            ,Empty,Work,

            """.write(to: file, atomically: true, encoding: .utf8)
        return file
    }

    func testTheImportSheetBeforeAndAfter() throws {
        let job = LibraryStore(core: core).importer(for: try export())
        let summary = try XCTUnwrap(job.summary, job.failure ?? "no dry run")
        XCTAssertEqual([summary.imported, summary.needsEdit, summary.skipped], [2, 1, 1])
        assertLabelled(ImportSheet(job: job, open: { _ in }), width: 680, height: 560, "import, dry run")
        XCTAssertTrue(job.run(), job.failure ?? "")
        assertLabelled(ImportSheet(job: job, open: { _ in }), width: 680, height: 560, "import, report")
    }

    func testTheImportSheetWithNothingToImport() throws {
        let file = cache.appendingPathComponent("empty.json")
        try FileManager.default.createDirectory(at: cache, withIntermediateDirectories: true)
        try "[]".write(to: file, atomically: true, encoding: .utf8)
        let job = LibraryStore(core: core).importer(for: file)
        assertLabelled(ImportSheet(job: job, open: { _ in }), width: 680, height: 560, "import, nothing")
    }
}

/// The palette's way into another app, which these tests never take.
private struct NoInserter: SnippetInserter {
    func insert(snippetId: String) async -> InsertFailure? { nil }
}

/// The command panel's way to the selection, which these tests never take.
private struct NoSelection: SelectionAccess {
    func read() async -> Result<String, SelectionFailure> { .success("") }
    func replace(with text: String) async -> SelectionFailure? { nil }
}

/// The form panel's way into the document, which these tests never take.
private final class NoRunner: ExpansionRunner, @unchecked Sendable {
    func run(_ action: SessionAction) {}
    func cancel(_ session: ExpansionSession) {}
}

/// A model that answers every block with `answer` at once, or, given none,
/// is still writing when the test looks.
private struct CannedModel: BlockRunner {
    let answer: String?

    func start(_ session: ExpansionSession, block: UInt32) async throws -> AiBlockRunProtocol {
        CannedRun(index: block, answer: answer)
    }
}

private final class CannedRun: AiBlockRunProtocol, @unchecked Sendable {
    private let index: UInt32
    private let reply: String?
    private let lock = NSLock()
    private var given = false

    init(index: UInt32, answer: String?) {
        self.index = index
        reply = answer
    }

    func next() async throws -> String? {
        guard let reply else {
            // Writing, for as long as the test looks.
            try await Task.sleep(for: .seconds(60))
            return nil
        }
        return lock.withLock {
            defer { given = true }
            return given ? nil : reply
        }
    }

    func cancel() {}
    func answer() -> String? { reply }
    func block() -> UInt32 { index }
    func cutShort() -> Bool { false }
    func model() -> String { "model-a" }
    func profile() -> String { "Example" }
    func sent() -> [AiContextSent] { [AiContextSent(kind: .fillins, bytes: 9)] }
    func text() -> String { lock.withLock { given ? reply ?? "" : "" } }
}
