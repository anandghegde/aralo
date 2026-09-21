import AppKit
import AraloKit
import SwiftUI

/// The editor: the snippet's fields, what its settings come to once the groups
/// above it are applied, anything the core has to say about the draft, and
/// what typing the abbreviation would produce.
///
/// The body is a plain text view for now. The highlighting editor is M2's next
/// task; nothing here depends on which one it is.
struct SnippetEditorView: View {
    let store: LibraryStore
    @Binding var editing: Editing
    let root: URL

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                fields
                Divider()
                body(of: editing)
                if !store.problems.isEmpty {
                    problems
                }
                Divider()
                settings
                Divider()
                preview
            }
            .padding(20)
        }
        .toolbar { toolbar }
        .navigationTitle(editing.draft.label.isEmpty ? "Untitled" : editing.draft.label)
        .navigationSubtitle(editing.path)
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

    private func body(of editing: Editing) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Body").font(.headline)
            TextEditor(text: $editing.draft.body)
                .font(.system(.body, design: .monospaced))
                .frame(minHeight: 160)
                .scrollContentBackground(.hidden)
                .padding(6)
                .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 6))
        }
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
                    Text(problem.message)
                    if let other = problem.conflictsWith {
                        Button("Show It") {
                            store.attempt { try $0.save() }
                            store.select(snippet: other)
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
