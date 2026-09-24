import AraloBridge
import Foundation
import Observation

/// A snippet that asks something before it expands: the form panel's model.
///
/// It drives one session — the form, then whatever the body wants from outside
/// it, then its AI blocks, then the plan — and knows nothing about windows. A
/// panel draws `fields`, writes into `answers`, shows `preview`, and calls
/// `submit` or `cancel`. On the AI step it draws `blocks` instead: the answers
/// stream into the preview, and Enter puts in what is there.
///
/// Nothing reaches the document until the last `submit`. Asking the core a
/// question changes nothing anywhere, so a user who opens a form and thinks
/// better of it has typed nothing.
@MainActor
@Observable
public final class FormSession {
    /// Which question the panel is on.
    public enum Step: Equatable, Sendable {
        /// The form's boxes.
        case form
        /// The AI blocks: models writing, or what they wrote, to read before
        /// it goes in.
        case blocks
    }

    public private(set) var step: Step = .form

    /// The boxes to put in front of the user, in the order the body names
    /// them. Empty when the snippet asked only for something from outside,
    /// which is a session with nothing to draw.
    public private(set) var fields: [FormField] = []

    /// What is in the boxes: the defaults to begin with, then whatever the
    /// user types. A name the form does not have is ignored by the core, so a
    /// panel may leave one in here.
    public var answers: [String: String] = [:] {
        didSet {
            guard answers != oldValue else { return }
            readPreview()
        }
    }

    /// What the snippet expands to with the answers as they stand: the
    /// expansion itself, run on a copy, minus the cursor and the keys. On the
    /// AI step, each block's text so far is in it.
    public private(set) var preview: String = ""

    /// Where each AI block's text is in `preview`, in UTF-16 code units: the
    /// stretches a panel marks as the model's. The rest is the snippet's own.
    public private(set) var marked: [BlockSpan] = []

    /// The snippet's AI blocks, once the session is on its AI step. Empty
    /// before that, and for a snippet with none.
    public private(set) var blocks: [AIBlock] = []

    /// The block whose text the user is editing, if any.
    public private(set) var editing: UInt32?

    /// The session is over: the plan has gone to the runner, or the user
    /// cancelled, or there was never a question to ask. A panel that finds
    /// this set on opening has nothing to show.
    public private(set) var isFinished = false

    /// Which snippet is expanding.
    public var snippetId: String { session.snippetId() }

    /// What the snippet is called, for the panel's heading: a user who typed
    /// an abbreviation and got a form should see which snippet asked.
    public let label: String

    /// A model is still writing, so there is nothing to put in yet.
    public var isWriting: Bool {
        blocks.contains { $0.status == .waiting || $0.status == .writing }
    }

    /// Whether `submit` can do anything now: not once the session is over,
    /// and not while a model is still writing.
    public var canSubmit: Bool {
        !isFinished && !(step == .blocks && isWriting)
    }

    /// Whether `submit` puts the text in, rather than moving on to the AI
    /// blocks. A panel gets out of the way first when it does: synthetic keys
    /// follow the keyboard.
    public var insertsOnSubmit: Bool {
        step == .blocks || !hasBlocks
    }

    @ObservationIgnored private let session: ExpansionSession
    @ObservationIgnored private let runner: ExpansionRunner
    @ObservationIgnored private let context: SessionContext
    @ObservationIgnored private let models: BlockRunner?
    @ObservationIgnored private let hasBlocks: Bool
    @ObservationIgnored private var runs: [UInt32: AiBlockRunProtocol] = [:]
    @ObservationIgnored private var task: Task<Void, Never>?
    /// Bumped by every round of asking and every stop, so an answer arriving
    /// for a round that was replaced or stopped is dropped.
    @ObservationIgnored private var generation = 0

    /// Takes over a session the engine started and asks it what it wants.
    ///
    /// `clipboard` is read only if the body asks for it: reading the
    /// pasteboard is not something to do on the chance that a snippet wanted
    /// it.
    public convenience init(
        session: ExpansionSession,
        runner: ExpansionRunner,
        label: String = "",
        clipboard: @escaping @MainActor () -> String? = { nil },
        models: BlockRunner? = nil
    ) {
        self.init(
            session: session, runner: runner, label: label,
            context: SessionContext(clipboard: clipboard), models: models
        )
    }

    /// The same, with everything the shell can fetch. Each kind is read only
    /// if the session asks for it. `models` answers the snippet's AI blocks;
    /// without it, every block puts in its fallback and the panel says why.
    public init(
        session: ExpansionSession,
        runner: ExpansionRunner,
        label: String = "",
        context: SessionContext,
        models: BlockRunner?
    ) {
        self.session = session
        self.runner = runner
        self.label = label
        self.context = context
        self.models = models
        hasBlocks = !session.aiBlocks().isEmpty
        advance(session.next())
    }

    /// The form is filled in, or the AI blocks are read: what the panel shows
    /// goes to the core, and the plan it hands back goes to the runner.
    ///
    /// When `insertsOnSubmit` is true the window must be out of the way
    /// before this is called. Synthetic keys follow the keyboard, so a plan
    /// run with the panel still up lands in the panel.
    public func submit() {
        guard canSubmit else { return }
        switch step {
        case .form:
            advance(session.submitForm(answers: answers))
        case .blocks:
            settle()
        }
    }

    /// Never mind. What the engine swallowed to open the panel goes back into
    /// the document, and nothing else happens. A model still writing is
    /// stopped.
    public func cancel() {
        guard !isFinished else { return }
        stopRuns()
        isFinished = true
        runner.cancel(session)
    }

    // MARK: - The AI step

    /// Asks the models again, for every block or for one.
    public func regenerate(_ index: UInt32? = nil) {
        guard step == .blocks, !isFinished, !isWriting else { return }
        editing = nil
        ask(index.map { [$0] } ?? blocks.map(\.index))
    }

    /// Stops the models that are writing. What arrived stays, to be read and
    /// edited; a block that got nothing puts in its fallback.
    public func stop() {
        guard step == .blocks, isWriting else { return }
        stopRuns()
        for block in blocks where block.status == .waiting || block.status == .writing {
            update(block.index) { block in
                if block.text.isEmpty {
                    block.status = .failed("you stopped it before the model answered")
                } else {
                    block.status = .written
                    block.cutShort = true
                }
            }
        }
    }

    /// Lets the user change a block's text before it goes in: the first
    /// block, when none is named.
    public func edit(_ index: UInt32? = nil) {
        guard step == .blocks, !isFinished, !isWriting else { return }
        editing = index ?? blocks.first?.index
    }

    /// The user's own text for a block. It goes in as written, in place of
    /// the model's answer or the fallback.
    public func setText(_ text: String, for index: UInt32) {
        guard step == .blocks, !isFinished else { return }
        update(index) { block in
            block.text = text
            block.status = .written
        }
    }

    /// Puts in a block's fallback instead of what the model wrote.
    public func useFallback(_ index: UInt32) {
        guard step == .blocks, !isFinished, !isWriting else { return }
        if editing == index { editing = nil }
        update(index) { $0.status = .failed("you chose the fallback") }
    }

    /// Answers the question the session asked, until it has none left.
    ///
    /// The context is fetched and handed back without asking the user: a
    /// snippet that wants the clipboard is not a snippet that wants a dialog
    /// about the clipboard.
    private func advance(_ action: SessionAction) {
        switch action {
        case .form(let asked):
            fields = asked.map(FormField.init)
            // The defaults are the answers until the user changes them, so
            // the preview shows what Enter would insert from the moment the
            // panel opens.
            answers = Dictionary(fields.map { ($0.name, $0.initial) }, uniquingKeysWith: { first, _ in first })
            readPreview()
        case .context(let kinds):
            advance(session.provideContext(values: supply(for: kinds)))
        case .ai(let asked):
            step = .blocks
            blocks = asked.map(AIBlock.init)
            ask(blocks.map(\.index))
        case .expand:
            isFinished = true
            runner.run(action)
        case .done:
            isFinished = true
        }
    }

    /// What the shell can fetch, of what was asked for, and nothing else.
    ///
    /// A pasteboard with no text on it is an empty clipboard, not a missing
    /// one: it was read, and what it holds as text is nothing. Copying an
    /// image must not leave `{{clipboard}}` sitting in the document.
    private func supply(for kinds: [ContextNeed]) -> ContextSupply {
        ContextSupply(
            clipboard: kinds.contains(.clipboard) ? context.clipboard() ?? "" : nil,
            selection: kinds.contains(.selection) ? context.selection() : nil,
            app: kinds.contains(.app) ? context.app() : nil,
            window: kinds.contains(.window) ? context.window() : nil
        )
    }

    /// Asks a model for each block in turn. A block with no one to ask puts
    /// in its fallback, and says why.
    private func ask(_ indexes: [UInt32]) {
        stopRuns()
        let current = generation
        for index in indexes {
            update(index) { block in
                block.status = .waiting
                block.text = ""
                block.cutShort = false
            }
        }
        task = Task { [weak self] in
            for index in indexes {
                guard let self, current == generation else { return }
                await write(index, round: current)
            }
        }
    }

    private func write(_ index: UInt32, round: Int) async {
        guard let models else {
            update(index) { $0.status = .failed("AI is not set up here") }
            return
        }
        update(index) { $0.status = .writing }
        do {
            let run = try await models.start(session, block: index)
            guard round == generation else {
                run.cancel()
                return
            }
            runs[index] = run
            update(index) { block in
                block.profile = run.profile()
                block.model = run.model()
                block.sent = run.sent()
            }
            while let piece = try await run.next() {
                guard round == generation else { return }
                update(index) { $0.text += piece }
            }
            guard round == generation else { return }
            runs[index] = nil
            let answer = run.answer()
            update(index) { block in
                block.cutShort = run.cutShort()
                if let answer {
                    block.text = answer
                    block.status = .written
                } else {
                    block.status = .failed("the model answered with nothing")
                }
            }
        } catch {
            guard round == generation else { return }
            runs[index] = nil
            update(index) { $0.status = .failed(error.reason) }
        }
    }

    /// Hands every block to the core, as the panel shows it: the text for a
    /// block that has one, the fallback for the rest. The last of them brings
    /// the plan.
    private func settle() {
        var action = SessionAction.done
        for block in blocks {
            switch block.status {
            case .written:
                action = session.answerBlock(index: block.index, text: block.text)
            case .failed(let reason):
                action = session.fallBack(index: block.index, reason: reason)
            case .waiting, .writing:
                return
            }
        }
        editing = nil
        advance(action)
    }

    private func stopRuns() {
        generation += 1
        task?.cancel()
        task = nil
        for run in runs.values {
            run.cancel()
        }
        runs = [:]
    }

    private func update(_ index: UInt32, _ change: (inout AIBlock) -> Void) {
        guard let position = blocks.firstIndex(where: { $0.index == index }) else { return }
        change(&blocks[position])
        readPreview()
    }

    private func readPreview() {
        var drafts: [UInt32: String] = [:]
        for block in blocks {
            switch block.status {
            case .writing, .written: drafts[block.index] = block.text
            case .waiting, .failed: break
            }
        }
        let rendered = session.previewBlocks(answers: answers, drafts: drafts)
        preview = rendered.text
        marked = rendered.blocks
    }
}
