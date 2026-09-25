import AraloBridge
import AraloKit
import SwiftUI

/// The editor's AI sheet: the answer to one action as it streams in, then read
/// against the text it replaces, chosen from, edited, and put in as one edit
/// the editor's own undo takes back (PRD A1).
struct AuthoringSheet: View {
    @Bindable var store: AuthoringStore
    /// Puts the draft in. False when the body moved on under the sheet.
    let replace: (String) -> Bool
    let close: () -> Void
    @State private var refusal: String?
    @FocusState private var editing: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider()
            content
            Divider()
            footer
        }
        .frame(width: 560, height: 420)
        .onExitCommand(perform: escape)
    }

    private var header: some View {
        HStack(spacing: 8) {
            Label(store.label, systemImage: "sparkles").font(.title3.weight(.semibold))
            Spacer()
            if store.phase == .running {
                ProgressView().controlSize(.small)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    @ViewBuilder private var content: some View {
        switch store.phase {
        case .asking:
            VStack(alignment: .leading, spacing: 8) {
                Text("What should the snippet say? The label is sent with this, and nothing else.")
                    .foregroundStyle(.secondary)
                TextField("A short, warm reply confirming the refund", text: $store.note, axis: .vertical)
                    .lineLimit(3...6)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(store.draftNow)
            }
            .padding(16)
            .frame(maxHeight: .infinity, alignment: .top)
        case .running:
            ScrollView {
                Text(store.streamed)
                    .font(.body.monospaced())
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
            }
        case .failed(let reason):
            VStack(spacing: 6) {
                Text(reason).font(.title3).multilineTextAlignment(.center)
                Text("Press R to try again, or Escape to close.").foregroundStyle(.secondary)
            }
            .padding(24)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .answered:
            answered
        }
    }

    private var answered: some View {
        VStack(alignment: .leading, spacing: 8) {
            if store.versions.count > 1 {
                Picker("Version", selection: Binding(get: { store.chosen }, set: { store.choose($0) })) {
                    ForEach(store.versions.indices, id: \.self) { index in
                        Text("Version \(index + 1)").tag(index)
                    }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .padding(.horizontal, 16)
                .padding(.top, 10)
            }
            if store.isEditing {
                TextEditor(text: $store.draft)
                    .font(.body.monospaced())
                    .focused($editing)
                    .padding(12)
                    .onAppear { editing = true }
            } else {
                ScrollView {
                    Text(DiffText.render(store.diff))
                        .font(.body.monospaced())
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(16)
                }
            }
        }
    }

    private var footer: some View {
        VStack(alignment: .leading, spacing: 6) {
            if !store.changes.isEmpty {
                Label(changesNote, systemImage: "curlybraces")
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if store.cutShort {
                Label("The model stopped before it finished. The end may be missing.",
                      systemImage: "exclamationmark.triangle")
                    .font(.callout)
            }
            if let refusal {
                Label(refusal, systemImage: "exclamationmark.triangle").font(.callout)
            }
            HStack(spacing: 8) {
                ManifestLabel(ContextManifest(
                    store.sent, profile: store.profile, model: store.model,
                    request: store.isDraft ? "the label and your note" : "the instruction"
                ))
                Spacer()
                Button("Cancel", action: close).keyboardShortcut(.cancelAction)
                buttons
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    @ViewBuilder private var buttons: some View {
        switch store.phase {
        case .asking:
            Button("Draft", action: store.draftNow)
                .keyboardShortcut(.defaultAction)
                .disabled(!store.canDraft)
        case .running:
            Button("Stop", action: store.cancel)
        case .failed:
            Button("Try Again", action: store.regenerate).keyboardShortcut("r", modifiers: [])
        case .answered where store.isEditing:
            // Return makes a new line in the editor.
            Button("Replace", action: put).keyboardShortcut(.return, modifiers: .command)
                .disabled(!store.canReplace)
        case .answered:
            Button("Edit", action: store.edit).keyboardShortcut("e", modifiers: [])
            Button("Regenerate", action: store.regenerate).keyboardShortcut("r", modifiers: [])
            Button("Replace", action: put).keyboardShortcut(.defaultAction)
                .disabled(!store.canReplace)
        }
    }

    /// Which placeholders the answer changed. A snippet body is a template:
    /// one the model wrote is one the snippet will expand.
    private var changesNote: String {
        let dropped = store.changes.filter { $0.change == .removed }.map(\.placeholder)
        let added = store.changes.filter { $0.change == .added }.map(\.placeholder)
        var parts: [String] = []
        if !dropped.isEmpty {
            parts.append("drops \(dropped.joined(separator: ", "))")
        }
        if !added.isEmpty {
            parts.append("adds \(added.joined(separator: ", "))")
        }
        return "This answer \(parts.joined(separator: " and ")). Check them before you replace."
    }

    private func put() {
        guard store.canReplace else { return }
        if replace(store.draft) {
            close()
        } else {
            refusal = "The body changed while the answer was being read, so nothing was replaced."
        }
    }

    /// Escape stops an answer that is coming, and otherwise closes the sheet.
    private func escape() {
        if store.phase == .running {
            store.cancel()
        } else {
            close()
        }
    }
}
