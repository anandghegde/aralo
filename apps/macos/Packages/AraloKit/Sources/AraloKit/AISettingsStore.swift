import AraloBridge
import Foundation
import Observation

/// The AI settings pane's model: the switch, local-only mode, the saved
/// profiles and the one being edited, with Test connection, the capability
/// probe, the model list and local-server detection.
///
/// Every rule lives in the core. This holds what is on screen and turns the
/// secure field into a key change; it checks nothing a second shell would have
/// to check again.
///
/// The key typed into the editor is held only until the profile is saved, and
/// is never read back: a saved profile says whether it has a key, not what it
/// is.
@MainActor
@Observable
public final class AISettingsStore {
    public private(set) var switches = AiSwitches(enabled: false, localOnly: false)
    public private(set) var profiles: [AiProfile] = []
    /// The providers the editor offers by name.
    public let presets: [AiProviderPreset]

    /// The profile on screen, saved or new. Nil when nothing is selected.
    public var editor: ProfileEditor? {
        didSet {
            // What was found out about one draft says nothing about another.
            guard editor?.target != oldValue?.target else { return }
            connection = nil
            models = []
        }
    }

    /// The last Test connection, for the draft on screen.
    public private(set) var connection: ConnectionOutcome?
    /// The models the draft's endpoint listed, for the model picker.
    public private(set) var models: [String] = []
    /// Model servers found on this Mac. Nil until Detect has run.
    public private(set) var localServers: [AiLocalServer]?

    /// What is in progress. One call at a time: each one sends a request, and
    /// the pane shows a spinner for it.
    public private(set) var busy: Busy?

    /// Why the last call failed, in the core's words, when it was not about a
    /// field in the editor.
    public private(set) var failure: String?

    public enum Busy: Equatable, Sendable {
        case testing, probing, listingModels, detecting
    }

    public enum ConnectionOutcome: Equatable, Sendable {
        case answered(AiConnectionReport)
        case failed(String)
    }

    @ObservationIgnored private let settings: AiProfiles

    public init(settings: AiProfiles) {
        self.settings = settings
        presets = aiProviderPresets()
        refresh()
    }

    /// Opens the settings in Aralo's state folder, with keys in the keychain.
    public static func openDefault() throws -> AISettingsStore {
        let path = AraloService.defaultCacheURL.appendingPathComponent("profiles.toml").path
        return AISettingsStore(settings: try AiProfiles.open(path: path, keys: .keychain))
    }

    /// Reads the file again.
    public func refresh() {
        attempt {
            try settings.reload()
            switches = try settings.switches()
            profiles = try settings.profiles()
        }
    }

    public func dismissFailure() {
        failure = nil
    }

    // MARK: - The switches

    public func setEnabled(_ enabled: Bool) {
        attempt {
            try settings.setSwitches(switches: AiSwitches(enabled: enabled, localOnly: switches.localOnly))
            switches = try settings.switches()
        }
    }

    public func setLocalOnly(_ localOnly: Bool) {
        attempt {
            try settings.setSwitches(switches: AiSwitches(enabled: switches.enabled, localOnly: localOnly))
            switches = try settings.switches()
        }
    }

    // MARK: - Choosing what to edit

    /// A new profile, filled in from a preset when one is given.
    public func newProfile(from preset: AiProviderPreset? = nil) {
        var editor = ProfileEditor()
        if let preset {
            editor.name = uniqueName(preset.name)
            editor.baseURL = preset.baseUrl
            editor.model = preset.defaultModel
            editor.keyPage = preset.keyPage.flatMap(URL.init(string:))
        }
        self.editor = editor
    }

    /// A new profile for a server Detect found. It needs no key.
    public func newProfile(from server: AiLocalServer) {
        var editor = ProfileEditor()
        editor.name = uniqueName(server.name)
        editor.baseURL = server.baseUrl
        editor.model = server.models.first ?? ""
        self.editor = editor
        models = server.models
    }

    /// Puts a saved profile in the editor.
    public func edit(_ name: String) {
        guard let profile = profiles.first(where: { $0.name == name }) else { return }
        editor = ProfileEditor(profile, keyPage: keyPage(for: profile.baseUrl))
    }

    public var selectedProfile: AiProfile? {
        guard let name = editor?.originalName else { return nil }
        return profiles.first { $0.name == name }
    }

    // MARK: - Saving

    /// Saves the draft on screen, then probes it when AI is on, so the pane
    /// shows what the endpoint can do. Returns false when the draft was not
    /// saved; the editor then says why beside the field.
    @discardableResult
    public func save() async -> Bool {
        guard var editor, busy == nil else { return false }
        let saved: AiProfile
        do {
            saved = try settings.save(draft: try editor.draft(), key: editor.keyChange)
        } catch let problem as ProfileEditor.Problem {
            editor.problem = problem
            self.editor = editor
            return false
        } catch {
            if case let AiBridgeError.Invalid(field, message) = error {
                editor.problem = ProfileEditor.Problem(field: field, message: message)
                self.editor = editor
            } else {
                failure = error.reason
            }
            return false
        }
        failure = nil
        refresh()
        self.editor = ProfileEditor(saved, keyPage: editor.keyPage)
        if saved.capabilities == nil, allowed(saved) {
            await probe(saved.name)
        }
        return true
    }

    public func delete(_ name: String) {
        attempt {
            try settings.delete(name: name)
            if editor?.originalName == name { editor = nil }
            profiles = try settings.profiles()
        }
    }

    public func makeDefault(_ name: String) {
        attempt {
            try settings.setDefault(name: name)
            profiles = try settings.profiles()
        }
    }

    // MARK: - Asking the endpoint

    /// One short chat with the draft on screen, saved or not.
    public func testConnection() async {
        guard let editor, let draft = check(editor) else { return }
        await run(.testing) {
            do {
                let report = try await self.settings.testConnection(draft: draft, key: editor.keyChange)
                self.connection = .answered(report)
            } catch {
                self.connection = .failed(error.reason)
            }
        }
    }

    /// Fills the model picker from the draft's endpoint.
    public func loadModels() async {
        guard let editor, let draft = check(editor) else { return }
        await run(.listingModels) {
            self.models = try await self.settings.listModels(draft: draft, key: editor.keyChange)
        }
    }

    /// Finds out what a saved profile's endpoint can do, and keeps the answer.
    public func probe(_ name: String) async {
        await run(.probing) {
            _ = try await self.settings.probeCapabilities(name: name)
            self.profiles = try self.settings.profiles()
        }
    }

    /// Looks for Ollama, LM Studio and the others on this Mac.
    public func detectLocalServers() async {
        await run(.detecting) {
            self.localServers = try await self.settings.detectLocalServers()
        }
    }

    // MARK: - Helpers

    /// Whether the policy lets a request reach this profile's endpoint.
    public func allowed(_ profile: AiProfile) -> Bool {
        switches.enabled && (!switches.localOnly || profile.isLocal)
    }

    private func run(_ work: Busy, _ body: @MainActor () async throws -> Void) async {
        guard busy == nil else { return }
        busy = work
        defer { busy = nil }
        do {
            try await body()
            failure = nil
        } catch {
            failure = error.reason
        }
    }

    private func attempt(_ work: () throws -> Void) {
        do {
            try work()
            failure = nil
        } catch {
            failure = error.reason
        }
    }

    /// The editor's draft, or nil with the problem shown in the editor.
    private func check(_ editor: ProfileEditor) -> AiProfileDraft? {
        do {
            return try editor.draft()
        } catch let problem as ProfileEditor.Problem {
            self.editor?.problem = problem
        } catch {
            failure = error.reason
        }
        return nil
    }

    private func uniqueName(_ base: String) -> String {
        let taken = Set(profiles.map { $0.name.lowercased() })
        guard taken.contains(base.lowercased()) else { return base }
        var number = 2
        while taken.contains("\(base) \(number)".lowercased()) { number += 1 }
        return "\(base) \(number)"
    }

    private func keyPage(for baseURL: String) -> URL? {
        presets.first { $0.baseUrl == baseURL }?.keyPage.flatMap(URL.init(string:))
    }
}

/// A profile as the editor holds it: text fields, the headers as lines, and
/// the secure field's contents.
public struct ProfileEditor: Equatable, Sendable {
    /// The name it was saved under; nil for a profile not saved yet.
    public var originalName: String?
    public var name = ""
    public var baseURL = ""
    public var model = ""
    /// One header a line, `Name: value`.
    public var headers = ""
    /// What the secure field holds. Blank keeps the saved key.
    public var key = ""
    /// Set to delete the saved key on save.
    public var removeKey = false
    /// Whether a key is saved for this profile.
    public private(set) var hasSavedKey = false
    /// Where to make a key for this provider, for the link beside the field.
    public var keyPage: URL?
    /// What stopped the last save, and the field it is about.
    public var problem: Problem?

    public struct Problem: Error, Equatable, Sendable {
        public let field: AiField
        public let message: String
    }

    /// What identifies the draft for results kept about it: a saved profile by
    /// its name, a new one by nothing.
    var target: String? { originalName }

    public init() {}

    init(_ profile: AiProfile, keyPage: URL?) {
        originalName = profile.name
        name = profile.name
        baseURL = profile.baseUrl
        model = profile.defaultModel
        headers = profile.headers.map { "\($0.name): \($0.value)" }.joined(separator: "\n")
        hasSavedKey = profile.hasKey
        self.keyPage = keyPage
    }

    /// What save and Test connection do with the key.
    public var keyChange: AiKeyChange {
        let typed = key.trimmingCharacters(in: .whitespacesAndNewlines)
        if !typed.isEmpty { return .set(key: typed) }
        if removeKey { return .remove }
        return originalName == nil ? .remove : .keep
    }

    /// The draft the core checks. Throws only for a header line that is not
    /// `Name: value`; everything else is the core's to judge.
    public func draft() throws -> AiProfileDraft {
        var parsed: [AiHeader] = []
        for line in headers.split(whereSeparator: \.isNewline) {
            let text = line.trimmingCharacters(in: .whitespaces)
            if text.isEmpty { continue }
            guard let colon = text.firstIndex(of: ":") else {
                throw Problem(field: .headers, message: "Write each header as Name: value.")
            }
            parsed.append(AiHeader(
                name: text[..<colon].trimmingCharacters(in: .whitespaces),
                value: text[text.index(after: colon)...].trimmingCharacters(in: .whitespaces)
            ))
        }
        return AiProfileDraft(
            originalName: originalName,
            name: name,
            baseUrl: baseURL,
            defaultModel: model,
            headers: parsed
        )
    }
}
