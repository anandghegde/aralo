import AraloBridge
@testable import AraloKit
import XCTest

/// The AI settings pane's model, against the real core with keys kept in
/// memory. Nothing here reaches a network: AI starts off, and the calls that
/// would send something are refused by the core's policy.
@MainActor
final class AISettingsStoreTests: XCTestCase {
    private var folder: URL!
    private var store: AISettingsStore!

    override func setUpWithError() throws {
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("aralo-ai-\(UUID().uuidString)")
        let path = folder.appendingPathComponent("profiles.toml").path
        store = AISettingsStore(settings: try AiProfiles.open(path: path, keys: .memory))
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: folder)
    }

    func testAPresetFillsTheEditorAndLinksToItsKeyPage() throws {
        let groq = try XCTUnwrap(store.presets.first { $0.id == "groq" })
        store.newProfile(from: groq)
        let editor = try XCTUnwrap(store.editor)
        XCTAssertEqual(editor.name, "Groq")
        XCTAssertEqual(editor.baseURL, "https://api.groq.com/openai/v1")
        XCTAssertEqual(editor.keyPage?.host, "console.groq.com")
        XCTAssertNil(editor.originalName)
    }

    func testSavingKeepsTheKeyOutOfTheFileAndOutOfTheEditor() async throws {
        store.newProfile(from: store.presets.first { $0.id == "openai" })
        store.editor?.key = "  sk-aralo-canary-swift-0123456789abcdef \n"
        let saved = await store.save()
        XCTAssertTrue(saved)

        let profile = try XCTUnwrap(store.profiles.first)
        XCTAssertTrue(profile.hasKey)
        XCTAssertTrue(profile.isDefault)
        // AI is off, so nothing was probed.
        XCTAssertNil(profile.capabilities)
        XCTAssertEqual(store.editor?.originalName, "OpenAI")
        XCTAssertEqual(store.editor?.key, "")
        XCTAssertEqual(store.editor?.keyChange, .keep)
        XCTAssertEqual(store.editor?.hasSavedKey, true)

        let file = try String(contentsOf: folder.appendingPathComponent("profiles.toml"), encoding: .utf8)
        XCTAssertFalse(file.contains("canary"), file)
    }

    func testAProblemIsShownBesideItsField() async throws {
        store.newProfile()
        store.editor?.name = "Proxy"
        store.editor?.baseURL = "https://proxy.example.com/v1"
        store.editor?.model = "m"
        store.editor?.headers = "Authorization: Bearer abc"
        let saved = await store.save()
        XCTAssertFalse(saved)
        XCTAssertEqual(store.editor?.problem?.field, .headers)
        XCTAssertTrue(store.profiles.isEmpty)

        store.editor?.headers = "not a header"
        await store.testConnection()
        XCTAssertEqual(store.editor?.problem?.field, .headers)
        XCTAssertNil(store.connection)
    }

    func testWithAIOffTestConnectionSaysSoAndDetectFails() async throws {
        store.newProfile(from: store.presets.first { $0.id == "openai" })
        await store.testConnection()
        guard case .failed(let message) = store.connection else {
            return XCTFail("expected a refusal, got \(String(describing: store.connection))")
        }
        XCTAssertTrue(message.contains("off"), message)

        await store.detectLocalServers()
        XCTAssertNil(store.localServers)
        XCTAssertNotNil(store.failure)
        XCTAssertNil(store.busy)
    }

    func testTheSwitchesAreSavedAndLocalOnlyBlocksARemoteProfile() async throws {
        store.newProfile(from: store.presets.first { $0.id == "openai" })
        await store.save()
        store.newProfile(from: store.presets.first { $0.id == "ollama" })
        await store.save()
        store.setEnabled(true)
        store.setLocalOnly(true)
        XCTAssertEqual(store.switches, AiSwitches(enabled: true, localOnly: true))

        let remote = try XCTUnwrap(store.profiles.first { $0.name == "OpenAI" })
        let local = try XCTUnwrap(store.profiles.first { $0.name == "Ollama" })
        XCTAssertFalse(store.allowed(remote))
        XCTAssertTrue(store.allowed(local))
    }

    func testRenameKeepsTheKeyAndDeleteClearsTheEditor() async throws {
        store.newProfile(from: store.presets.first { $0.id == "openai" })
        store.editor?.key = "sk-aralo-canary-swift-0123456789abcdef"
        await store.save()
        store.editor?.name = "Work"
        await store.save()
        XCTAssertEqual(store.profiles.map(\.name), ["Work"])
        XCTAssertEqual(store.profiles.first?.hasKey, true)

        store.newProfile(from: store.presets.first { $0.id == "openai" })
        XCTAssertEqual(store.editor?.name, "OpenAI")
        store.edit("Work")
        store.delete("Work")
        XCTAssertNil(store.editor)
        XCTAssertTrue(store.profiles.isEmpty)
    }

    func testASecondProfileFromOnePresetGetsItsOwnName() async throws {
        let openai = store.presets.first { $0.id == "openai" }
        store.newProfile(from: openai)
        await store.save()
        store.newProfile(from: openai)
        XCTAssertEqual(store.editor?.name, "OpenAI 2")
    }
}
