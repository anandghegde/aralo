import AppKit
import AraloKit
import SwiftUI

/// The editor: the snippet's fields, what its settings come to once the groups
/// above it are applied, anything the core has to say about the draft, and
/// what typing the abbreviation would produce, and a field to type it into.
///
/// The body is [`BodyEditor`], which draws the core's reading of it. Nothing
/// else here knows about placeholders.
struct SnippetEditorView: View {
    let store: LibraryStore
    @Binding var editing: Editing
    let root: URL
    /// Opens the resolver for a conflict copy.
    let resolve: (String) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                if let copy = store.conflict(for: editing.id)?.copy {
                    conflictBanner(copy: copy)
                }
                fields
                Divider()
                bodyEditor
                if !store.problems.isEmpty {
                    problems
                }
                Divider()
                settings
                Divider()
                preview
                TestFieldView(store: store, editing: editing)
            }
            .padding(20)
        }
        .toolbar { toolbar }
        .navigationTitle(editing.draft.label.isEmpty ? "Untitled" : editing.draft.label)
        .navigationSubtitle(editing.path)
    }

    // MARK: - Sync conflict

    /// A sync client left a second version of this snippet that does not
    /// merge. The one on screen is still the one that expands.
    private func conflictBanner(copy: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: "exclamationmark.arrow.triangle.2.circlepath").foregroundStyle(.orange)
                .accessibilityHidden(true)
            Text(
                "Sync left another version of this snippet, and the two changed the same thing. "
                    + "This one expands until you choose."
            )
                .fixedSize(horizontal: false, vertical: true)
            Spacer()
            Button("Choose\u{2026}") { resolve(copy) }
        }
        .font(.callout)
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 6))
    }

    // MARK: - Fields

    private var fields: some View {
        VStack(alignment: .leading, spacing: 12) {
            LabeledContent("Label") {
                TextField("Label", text: $editing.draft.label)
                    .textFieldStyle(.roundedBorder)
                    .labelsHidden()
            }
            LabeledContent("Abbreviations") {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(editing.draft.abbreviations.indices, id: \.self) { index in
                        HStack(spacing: 6) {
                            TextField("Abbreviation", text: $editing.draft.abbreviations[index])
                                .textFieldStyle(.roundedBorder)
                                .font(.body.monospaced())
                            Button {
                                editing.draft.abbreviations.remove(at: index)
                            } label: {
                                Image(systemName: "minus.circle")
                            }
                            .accessibilityLabel("Remove Abbreviation")
                            .buttonStyle(.borderless)
                            .help("Remove this abbreviation")
                        }
                    }
                    Button {
                        editing.draft.abbreviations.append("")
                    } label: {
                        Label("Add Abbreviation", systemImage: "plus")
                    }
                    .buttonStyle(.borderless)
                    Text("A snippet can answer to more than one. Any of them expands it.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            LabeledContent("Tags") {
                VStack(alignment: .leading, spacing: 4) {
                    TextField("Tags", text: tags).textFieldStyle(.roundedBorder).labelsHidden()
                    Text("Separated by commas. The sidebar's search finds them.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private var tags: Binding<String> {
        Binding(
            get: { editing.draft.tags.joined(separator: ", ") },
            set: { text in
                editing.draft.tags = text
                    .split(separator: ",")
                    .map { $0.trimmingCharacters(in: .whitespaces) }
                    .filter { !$0.isEmpty }
            }
        )
    }

    private var bodyEditor: some View {
        BodyEditor(store: store, text: $editing.draft.body, label: editing.draft.label)
    }

    // MARK: - Advice

    /// What the core has to say about the draft. None of it stops a save: the
    /// folder is the user's, and a snippet that clashes is still a file they
    /// wrote on purpose.
    private var problems: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(store.problems.enumerated()), id: \.offset) { _, problem in
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                        .accessibilityLabel("Problem")
                    Text(problem.message)
                    if let other = problem.conflictsWith {
                        Button("Show It") {
                            if store.leave() { store.select(snippet: other) }
                        }
                        .buttonStyle(.link)
                    }
                }
                .font(.callout)
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 6))
    }

    // MARK: - Settings

    /// Each of these can be left to the groups above. The empty choice says
    /// what they make of it, so nothing on screen is ever a blank.
    private var settings: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Settings").font(.headline)
            InheritedPicker(
                title: "Expands",
                value: $editing.draft.trigger,
                inherited: editing.resolved.trigger,
                names: [
                    (.immediate, "As soon as it is typed"),
                    (.delimiter, "When a delimiter follows")
                ]
            )
            InheritedPicker(
                title: "Case",
                value: $editing.draft.case,
                inherited: editing.resolved.case,
                names: [
                    (.exact, "Match exactly"),
                    (.ignore, "Ignore case"),
                    (.adaptive, "Follow how it is typed")
                ]
            )
            InheritedPicker(
                title: "Whole word only",
                value: $editing.draft.wholeWord,
                inherited: editing.resolved.wholeWord,
                names: yesNo
            )
            InheritedPicker(
                title: "Keep the delimiter",
                value: $editing.draft.keepDelimiter,
                inherited: editing.resolved.keepDelimiter,
                names: yesNo
            )
            InheritedPicker(
                title: "Switched on",
                value: $editing.draft.enabled,
                inherited: editing.resolved.enabled,
                names: yesNo
            )
            if !editing.resolved.enabled, editing.draft.enabled != false {
                Text("A group above this one is switched off, so it will not expand.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var yesNo: [(Bool, String)] { [(true, "Yes"), (false, "No")] }

    // MARK: - Preview

    private var preview: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Preview").font(.headline)
            Text(store.preview(of: editing.draft.body))
                .font(.system(.body, design: .monospaced))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(8)
                .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 6))
            Text("What typing the abbreviation would produce.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    // MARK: - Toolbar

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .status) {
            Text(editing.isDirty ? "Edited" : "Saved")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        ToolbarItem {
            Button("Revert") { store.revert() }
                .disabled(!editing.isDirty)
        }
        ToolbarItem {
            Button("Save") { store.attempt { try $0.save() } }
                .keyboardShortcut("s")
                .disabled(!editing.isDirty)
        }
        ToolbarItem {
            Button {
                store.attempt { try $0.save() }
                NSWorkspace.shared.activateFileViewerSelecting([root.appendingPathComponent(editing.path)])
            } label: {
                Label("Reveal in Finder", systemImage: "folder")
            }
            .help("Show the file this snippet is")
        }
    }
}

/// A setting that can be left to the groups above. The first choice is
/// "inherited", and it says what that comes to.
private struct InheritedPicker<Value: Hashable>: View {
    let title: String
    @Binding var value: Value?
    /// What the groups above make of it, for the inherited choice to show.
    let inherited: Value
    /// Every choice and what to call it, in the order to offer them.
    let names: [(Value, String)]

    var body: some View {
        Picker(title, selection: $value) {
            Text("Inherited \u{2014} \(name(of: inherited))").tag(Value?.none)
            Divider()
            ForEach(names, id: \.0) { choice in
                Text(choice.1).tag(Value?.some(choice.0))
            }
        }
        .pickerStyle(.menu)
        .frame(maxWidth: 420, alignment: .leading)
    }

    private func name(of value: Value) -> String {
        names.first { $0.0 == value }?.1 ?? ""
    }
}
