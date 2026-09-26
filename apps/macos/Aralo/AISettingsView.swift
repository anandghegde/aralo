import AraloBridge
import AraloKit
import SwiftUI

/// The AI tab of Settings: the switch and local-only mode on top, the profiles
/// on the left, and the one selected on the right with its key, Test
/// connection and what the probe found. Everything it does is a call on
/// `AISettingsStore`; what lives here is how it looks.
struct AISettingsView: View {
    @Bindable var store: AISettingsStore

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            switches
            Divider()
            HSplitView {
                sidebar.frame(minWidth: 200, idealWidth: 220, maxWidth: 280)
                detail.frame(minWidth: 420, maxWidth: .infinity, maxHeight: .infinity)
            }
            if let failure = store.failure {
                Label(failure, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(20)
        .frame(minWidth: 700, idealWidth: 760, minHeight: 520, idealHeight: 580)
    }

    // MARK: - The switches

    private var switches: some View {
        VStack(alignment: .leading, spacing: 6) {
            Toggle("Use AI", isOn: Binding(get: { store.switches.enabled }, set: { store.setEnabled($0) }))
                .toggleStyle(.switch)
                .font(.headline)
            Text("Off until you turn it on. While it is off, Aralo sends nothing to any model, here or anywhere.")
                .font(.callout)
                .foregroundStyle(.secondary)
            Toggle(
                "Only model servers on this Mac",
                isOn: Binding(get: { store.switches.localOnly }, set: { store.setLocalOnly($0) })
            )
            .disabled(!store.switches.enabled)
        }
    }

    // MARK: - The list

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 8) {
            List(selection: Binding(get: { store.editor?.originalName }, set: { name in
                if let name { store.edit(name) }
            })) {
                ForEach(store.profiles, id: \.name) { profile in
                    row(profile).tag(Optional(profile.name))
                        .contextMenu {
                            Button("Make Default") { store.makeDefault(profile.name) }
                                .disabled(profile.isDefault)
                            Button("Delete", role: .destructive) { store.delete(profile.name) }
                        }
                }
            }
            .listStyle(.sidebar)
            .overlay {
                if store.profiles.isEmpty {
                    Text("Add a provider, or look for a model server on this Mac.")
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .padding()
                }
            }
            HStack {
                Menu {
                    ForEach(store.presets, id: \.id) { preset in
                        Button(preset.name) { store.newProfile(from: preset) }
                    }
                    Divider()
                    Button("Other OpenAI-Compatible Endpoint") { store.newProfile() }
                } label: {
                    Image(systemName: "plus")
                }
                .accessibilityLabel("Add a Provider")
                .menuStyle(.borderlessButton)
                .fixedSize()
                .help("Add a provider")
                Button {
                    if let name = store.editor?.originalName { store.delete(name) }
                } label: {
                    Image(systemName: "minus")
                }
                .accessibilityLabel("Delete Profile")
                .buttonStyle(.borderless)
                .disabled(store.editor?.originalName == nil)
                .help("Delete the selected profile and its key")
                Spacer()
                Button("Detect") { Task { await store.detectLocalServers() } }
                    .disabled(store.busy != nil)
                    .help("Look for Ollama, LM Studio, llama.cpp and vLLM on this Mac")
            }
            localServers
        }
    }

    private func row(_ profile: AiProfile) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(profile.name)
                Text(profile.isLocal ? "This Mac" : (URL(string: profile.baseUrl)?.host ?? profile.baseUrl))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if profile.isDefault {
                Text("Default").font(.caption2).foregroundStyle(.secondary)
            }
            if !store.allowed(profile) {
                // SwiftUI would read this symbol as "Stop".
                Image(systemName: "nosign")
                    .foregroundStyle(.secondary)
                    .help(store.switches.enabled ? "Only model servers on this Mac are allowed" : "AI is off")
                    .accessibilityLabel(store.switches.enabled ? "Not allowed: not on this Mac" : "AI is off")
            }
        }
    }

    @ViewBuilder
    private var localServers: some View {
        if store.busy == .detecting {
            ProgressView().controlSize(.small).accessibilityLabel("Looking for model servers")
        } else if let servers = store.localServers {
            if servers.isEmpty {
                Text("No model server is running on this Mac.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            ForEach(servers, id: \.baseUrl) { server in
                HStack {
                    Text("\(server.name) · \(server.models.count) models").font(.caption)
                    Spacer()
                    Button("Add") { store.newProfile(from: server) }.controlSize(.small)
                }
            }
        }
    }

    // MARK: - The editor

    @ViewBuilder
    private var detail: some View {
        if store.editor != nil {
            ProfileEditorView(store: store)
        } else {
            ContentUnavailableView(
                "No Profile Selected",
                systemImage: "sparkles",
                description: Text("A profile is a provider, its address, a model and your key for it.")
            )
        }
    }
}

/// The selected profile's fields, key, Test connection and capabilities.
private struct ProfileEditorView: View {
    @Bindable var store: AISettingsStore

    var body: some View {
        if let binding = Binding($store.editor) {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    fields(binding)
                    key(binding)
                    actions(binding.wrappedValue)
                    capabilities
                }
                .padding(.leading, 16)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private func fields(_ editor: Binding<ProfileEditor>) -> some View {
        Form {
            TextField("Name", text: editor.name)
            problem(for: .name, in: editor.wrappedValue)
            TextField("Address", text: editor.baseURL, prompt: Text("https://api.example.com/v1"))
            problem(for: .baseUrl, in: editor.wrappedValue)
            HStack {
                TextField("Model", text: editor.model)
                if !store.models.isEmpty {
                    Picker("Model from the List", selection: editor.model) {
                        ForEach(store.models, id: \.self) { Text($0).tag($0) }
                    }
                    .labelsHidden()
                    .fixedSize()
                }
                Button("List Models") { Task { await store.loadModels() } }
                    .disabled(store.busy != nil)
            }
            problem(for: .model, in: editor.wrappedValue)
            LabeledContent("Headers") {
                TextEditor(text: editor.headers)
                    .accessibilityLabel("Headers")
                    .font(.body.monospaced())
                    .frame(height: 44)
                    .overlay(alignment: .topLeading) {
                        if editor.wrappedValue.headers.isEmpty {
                            Text("X-Title: Aralo").foregroundStyle(.tertiary).padding(.leading, 5)
                                .allowsHitTesting(false)
                                .accessibilityHidden(true)
                        }
                    }
            }
            problem(for: .headers, in: editor.wrappedValue)
        }
        .formStyle(.columns)
    }

    private func key(_ editor: Binding<ProfileEditor>) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                SecureField(
                    "API key",
                    text: editor.key,
                    prompt: Text(editor.wrappedValue.hasSavedKey ? "Saved in the keychain" : "None")
                )
                if editor.wrappedValue.hasSavedKey {
                    Toggle("Remove", isOn: editor.removeKey)
                        .disabled(!editor.wrappedValue.key.isEmpty)
                }
            }
            if let page = editor.wrappedValue.keyPage {
                // Guided setup: the page the key is made on, one click away.
                Link("Get a key from \(page.host ?? "the provider")\u{2026}", destination: page)
                    .font(.callout)
            }
            Text("Keys are kept in your keychain and sent only to this address.")
                .font(.caption)
                .foregroundStyle(.secondary)
            problem(for: .key, in: editor.wrappedValue)
        }
    }

    private func actions(_ editor: ProfileEditor) -> some View {
        HStack(spacing: 10) {
            Button("Test Connection") { Task { await store.testConnection() } }
                .disabled(store.busy != nil)
            if store.busy == .testing || store.busy == .listingModels {
                ProgressView().controlSize(.small)
                    .accessibilityLabel(store.busy == .testing ? "Testing the connection" : "Listing models")
            }
            connection
            Spacer()
            Button(editor.originalName == nil ? "Add" : "Save") { Task { await store.save() } }
                .keyboardShortcut(.defaultAction)
                .disabled(store.busy != nil)
        }
    }

    @ViewBuilder
    private var connection: some View {
        switch store.connection {
        case .answered(let report):
            Label("\(report.model) answered in \(report.firstTokenMs) ms", systemImage: "checkmark.circle.fill")
                .foregroundStyle(.green)
                .help(report.reply)
        case .failed(let message):
            Label(message, systemImage: "xmark.octagon.fill")
                .foregroundStyle(.red)
                .lineLimit(3)
                .help(message)
        case nil:
            EmptyView()
        }
    }

    @ViewBuilder
    private var capabilities: some View {
        if let profile = store.selectedProfile {
            GroupBox {
                if let found = profile.capabilities {
                    Grid(alignment: .leading, verticalSpacing: 4) {
                        capability("Model list", found.modelsRoute)
                        capability("Streaming", found.streaming)
                        capability("System prompt", found.systemPrompt)
                        capability("JSON output", found.jsonOutput)
                        capability("Embeddings", found.embeddings)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    Text("Not checked yet.").foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            } label: {
                HStack {
                    Text("What this endpoint can do")
                    Spacer()
                    if store.busy == .probing {
                        ProgressView().controlSize(.small).accessibilityLabel("Checking")
                    }
                    Button("Check Again") { Task { await store.probe(profile.name) } }
                        .controlSize(.small)
                        .disabled(store.busy != nil || !store.allowed(profile))
                }
            }
        }
    }

    private func capability(_ title: String, _ check: AiCheck) -> some View {
        GridRow {
            Text(title)
            switch check {
            case .yes:
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green).accessibilityLabel("Yes")
            case .no(let reason):
                Label(reason, systemImage: "xmark.circle.fill").foregroundStyle(.orange).lineLimit(2)
            case .notChecked(let reason):
                Label(reason, systemImage: "minus.circle").foregroundStyle(.secondary).lineLimit(2)
            }
        }
    }

    @ViewBuilder
    private func problem(for field: AiField, in editor: ProfileEditor) -> some View {
        if let problem = editor.problem, problem.field == field {
            Text(problem.message).font(.caption).foregroundStyle(.red)
        }
    }
}
