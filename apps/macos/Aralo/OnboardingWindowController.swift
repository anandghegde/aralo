import AppKit
import AraloKit
import Observation
import SwiftUI

/// What the first-run window shows. It polls, because macOS gives no callback
/// when the user flips a switch in System Settings.
@MainActor
@Observable
final class OnboardingModel {
    private(set) var flow: OnboardingFlow
    private(set) var facts: OnboardingFlow.Facts
    private(set) var example: OnboardingExample?
    private(set) var library: OnboardingLibrary
    private(set) var pauseShortcut: String?
    var trialText = ""

    @ObservationIgnored private let service: AraloService
    @ObservationIgnored private var timer: Timer?
    @ObservationIgnored var onFinish: (() -> Void)?
    /// Opens the library window's import panel. What comes in shows up on
    /// the library screen at the next poll.
    @ObservationIgnored var onImport: (() -> Void)?
    /// Opens the AI tab of Settings. The flow never turns AI on itself.
    @ObservationIgnored var onSetUpAI: (() -> Void)?
    /// Opens the Library tab of Settings, where the folder is chosen or moved.
    @ObservationIgnored var onChooseLibrary: (() -> Void)?

    init(service: AraloService, startingAt step: OnboardingStep, returning: Bool = false) {
        self.service = service
        flow = OnboardingFlow(startingAt: step, returning: returning)
        facts = Self.facts(of: service)
        example = service.onboardingExample
        library = service.onboardingLibrary
        pauseShortcut = service.pauseShortcut?.display
    }

    private static func facts(of service: AraloService) -> OnboardingFlow.Facts {
        OnboardingFlow.Facts(permissions: Permissions.status, tapRunning: service.isTapRunning)
    }

    var canContinue: Bool { flow.canContinue(facts) }
    var worked: Bool { example?.isExpanded(in: trialText) ?? false }

    func startPolling() {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
    }

    func stopPolling() {
        timer?.invalidate()
        timer = nil
    }

    private func refresh() {
        let latest = Self.facts(of: service)
        if latest != facts { facts = latest }
        // An import can change both the count and which snippet to try.
        let latestLibrary = service.onboardingLibrary
        if latestLibrary != library {
            library = latestLibrary
            example = service.onboardingExample
        }
        if example == nil { example = service.onboardingExample }
    }

    func proceed() {
        refresh()
        if !flow.advance(facts), flow.isLast {
            onFinish?()
        }
    }

    func back() { flow.back() }

    func importSnippets() { onImport?() }
    func setUpAI() { onSetUpAI?() }
    func chooseLibrary() { onChooseLibrary?() }
    func showLibraryInFinder() { NSWorkspace.shared.activateFileViewerSelecting([library.folder]) }

    /// The first click shows the system prompt. macOS shows it only once, so
    /// every click also opens the right pane of System Settings.
    func grant(_ pane: Permissions.Pane) {
        Permissions.request(pane)
        Permissions.openSettings(pane)
    }
}

@MainActor
final class OnboardingWindowController: NSWindowController, NSWindowDelegate {
    static let completedKey = "onboardingCompleted"

    private let model: OnboardingModel

    var onImport: (() -> Void)? {
        get { model.onImport }
        set { model.onImport = newValue }
    }

    var onSetUpAI: (() -> Void)? {
        get { model.onSetUpAI }
        set { model.onSetUpAI = newValue }
    }

    var onChooseLibrary: (() -> Void)? {
        get { model.onChooseLibrary }
        set { model.onChooseLibrary = newValue }
    }

    init(service: AraloService, startingAt step: OnboardingStep, returning: Bool = false) {
        model = OnboardingModel(service: service, startingAt: step, returning: returning)
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 520, height: 420),
            styleMask: [.titled, .closable, .fullSizeContentView], backing: .buffered, defer: false
        )
        window.title = "Welcome to Aralo"
        window.titlebarAppearsTransparent = true
        window.isReleasedWhenClosed = false
        window.contentViewController = NSHostingController(rootView: OnboardingView(model: model))
        super.init(window: window)
        window.delegate = self
        model.onFinish = { [weak self] in
            UserDefaults.standard.set(true, forKey: Self.completedKey)
            self?.close()
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used from a nib")
    }

    func show() {
        model.startPolling()
        window?.center()
        // An agent app is never active on its own; the test field needs the keyboard.
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        model.stopPolling()
    }
}

/// Internal rather than private so the accessibility audit can draw it.
struct OnboardingView: View {
    @Bindable var model: OnboardingModel

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            page
            Spacer(minLength: 0)
            HStack {
                if model.flow.step != .privacy {
                    Button("Back") { model.back() }
                }
                Spacer()
                Text("Step \(model.flow.position) of \(model.flow.steps.count)")
                    .foregroundStyle(.secondary)
                Button(model.flow.isLast ? "Done" : "Continue") { model.proceed() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!model.canContinue)
            }
        }
        .padding(28)
        .frame(width: 520, height: 420)
    }

    @ViewBuilder private var page: some View {
        switch model.flow.step {
        case .privacy:
            privacy
        case .accessibility:
            PermissionPage(
                title: "Allow Accessibility",
                why: """
                    Accessibility access lets Aralo remove the abbreviation you typed and put the \
                    expansion in its place.
                    """,
                how: """
                    macOS will show a prompt that sends you to System Settings. Switch Aralo on \
                    under Privacy & Security › Accessibility. This window notices when you have.
                    """,
                granted: model.facts.permissions.accessibility
            ) { model.grant(.accessibility) }
        case .inputMonitoring:
            PermissionPage(
                title: "Allow Input Monitoring",
                why: "Input Monitoring lets Aralo notice that you typed an abbreviation, in any app.",
                how: """
                    macOS will ask whether Aralo may receive keystrokes. Switch Aralo on under \
                    Privacy & Security › Input Monitoring. macOS may ask you to reopen Aralo.
                    """,
                granted: model.canContinue
            ) { model.grant(.inputMonitoring) }
        case .library:
            library
        case .aiOptIn:
            aiOptIn
        case .tryIt:
            tryIt
        }
    }

    private var privacy: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("What Aralo reads, and what it never keeps").font(.title2).bold()
            Text("""
                To expand an abbreviation, Aralo has to notice that you typed it. So it sees your keys. \
                This is all it does with them:
                """)
            Label("It remembers the last 64 characters, in memory only.", systemImage: "memorychip")
            Label("It forgets them when you click, switch apps, move the caret or pause.", systemImage: "eraser")
            Label("It never writes them to disk and never sends them anywhere.", systemImage: "wifi.slash")
            Label("Password fields are invisible to it, and password managers are excluded.", systemImage: "lock")
            Text("Your snippets are plain files in a folder you own. There is no account and no telemetry.")
                .foregroundStyle(.secondary)
        }
    }

    private var library: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Your snippets").font(.title2).bold()
            Text(model.library.summary)
            HStack(spacing: 8) {
                Label(model.library.displayPath, systemImage: "folder")
                    .lineLimit(1).truncationMode(.middle)
                    .accessibilityLabel("Library folder: \(model.library.displayPath)")
                Button("Show in Finder") { model.showLibraryInFinder() }
            }
            Text("""
                Each snippet is a plain file in this folder. Keep the folder in iCloud Drive, Dropbox or \
                a Git repository to have the same snippets on every Mac.
                """)
                .foregroundStyle(.secondary)
            HStack {
                Button("Move or Choose Folder…") { model.chooseLibrary() }
                Text("in Settings › Library, now or later.").foregroundStyle(.secondary)
            }
            Divider()
            Text("Coming from another app? Bring its snippets in from an export.")
            Button("Import Snippets…") { model.importSnippets() }
        }
    }

    private var aiOptIn: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("AI, if you want it").font(.title2).bold()
            Text("""
                Aralo can draft text and run commands on selected text with a model you choose: one on \
                this Mac, or a provider you have a key for.
                """)
            Label("AI is off until you turn it on.", systemImage: "power")
            Label("Nothing is sent to a model until you do.", systemImage: "wifi.slash")
            Label("Before anything is sent, Aralo shows what it is and where it goes.", systemImage: "eye")
            HStack {
                Button("Set Up AI…") { model.setUpAI() }
                Text("or press Continue to leave it off.").foregroundStyle(.secondary)
            }
        }
    }

    private var tryIt: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Try it").font(.title2).bold()
            if let example = model.example {
                Text("Click in the box, type **\(example.abbreviation)** and press Space.")
            } else {
                Text("""
                    Your library has no snippet to try yet. Add one to the library folder, then type \
                    its abbreviation here.
                    """)
            }
            TextEditor(text: $model.trialText)
                .font(.title3)
                .accessibilityLabel("Try-it field")
                .frame(height: 120)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(.quaternary))
            if model.worked {
                Label("It works. Aralo now does this in every app.", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
            } else if !model.facts.tapRunning {
                Label(
                    "Aralo cannot see keys yet. Go back and check both permissions.",
                    systemImage: "exclamationmark.triangle"
                )
                .foregroundStyle(.orange)
            }
            Text(
                model.pauseShortcut.map { "Pause from anywhere with \($0), or from the menu bar icon." }
                    ?? "Pause at any time from the menu bar icon."
            )
            .foregroundStyle(.secondary)
        }
    }
}

struct PermissionPage: View {
    let title: String
    let why: String
    let how: String
    let granted: Bool
    let grant: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.title2).bold()
            Text(why)
            Text(how).foregroundStyle(.secondary)
            if granted {
                Label("Granted", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
            } else {
                Button("Open System Settings…", action: grant)
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small).accessibilityHidden(true)
                    Text("Waiting for you to switch it on…").foregroundStyle(.secondary)
                }
            }
        }
    }
}
