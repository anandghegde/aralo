import AppKit
import AraloKit
import SwiftUI

/// The main window: the group tree, the snippets in the selected group, and
/// the one being edited. Every command here is a call on `LibraryStore`; what
/// lives in this file is only how it looks.
struct LibraryView: View {
    @Bindable var store: LibraryStore
    /// Where a file lives on disk, for "Reveal in Finder".
    let root: URL

    @State private var renaming: Rename?
    @State private var deletingGroup: [String]?

    var body: some View {
        NavigationSplitView {
            GroupSidebar(store: store, selection: groupSelection, rename: startRename, delete: askDelete)
                .navigationSplitViewColumnWidth(min: 180, ideal: 220)
        } content: {
            SnippetList(store: store, selection: snippetSelection, root: root)
                .navigationSplitViewColumnWidth(min: 240, ideal: 300)
        } detail: {
            detail
        }
        .frame(minWidth: 860, minHeight: 520)
        .alert("That did not work", isPresented: failureShown) {
            Button("OK") { store.dismissFailure() }
        } message: {
            Text(store.failure ?? "")
        }
        .sheet(item: $renaming) { rename in
            RenameSheet(rename: rename) { name in
                store.attempt { try $0.rename(group: rename.path, to: name) }
            }
        }
        .confirmationDialog(
            deletingGroup.map { "Delete \u{201C}\($0.last ?? "")\u{201D}?" } ?? "",
            isPresented: deleteShown,
            titleVisibility: .visible
        ) {
            Button("Move to Trash", role: .destructive) {
                guard let path = deletingGroup else { return }
                store.attempt { try $0.delete(group: path) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(deleteWarning)
        }
    }

    @ViewBuilder
    private var detail: some View {
        if let editing = Binding($store.editing) {
            SnippetEditorView(store: store, editing: editing, root: root)
        } else {
            ContentUnavailableView(
                "No Snippet Selected",
                systemImage: "text.cursor",
                description: Text("Pick one from the list, or press \u{2318}N to write a new one.")
            )
        }
    }

    // MARK: - Selection

    /// Moving off a snippet writes what was typed into it. The file is the
    /// record: leaving an edit behind only in the window would be the
    /// surprise, not saving it.
    private func leave() {
        store.attempt { try $0.save() }
    }

    private var groupSelection: Binding<String?> {
        Binding(
            get: { store.selectedGroup.joined(separator: "/") },
            set: { id in
                leave()
                store.select(group: GroupNode.path(of: id ?? ""))
            }
        )
    }

    private var snippetSelection: Binding<String?> {
        Binding(
            get: { store.selectedSnippet },
            set: { id in
                guard id != store.selectedSnippet else { return }
                leave()
                store.select(snippet: id)
            }
        )
    }

    // MARK: - Sheets

    private var failureShown: Binding<Bool> {
        Binding(get: { store.failure != nil }, set: { shown in if !shown { store.dismissFailure() } })
    }

    private var deleteShown: Binding<Bool> {
        Binding(get: { deletingGroup != nil }, set: { shown in if !shown { deletingGroup = nil } })
    }

    private func startRename(_ path: [String]) {
        renaming = Rename(path: path, name: path.last ?? "")
    }

    private func askDelete(_ path: [String]) {
        deletingGroup = path
    }

    /// Says how much is about to go, because a group takes everything inside
    /// it and the folder is the user's own.
    private var deleteWarning: String {
        guard let path = deletingGroup else { return "" }
        let count = store.contents(of: path)
        let snippets = count == 1 ? "1 snippet" : "\(count) snippets"
        return "The folder and the \(snippets) in it go to the Trash. You can put them back from there."
    }

    struct Rename: Identifiable {
        var path: [String]
        var name: String
        var id: String { path.joined(separator: "/") }
    }
}

private struct RenameSheet: View {
    let rename: LibraryView.Rename
    let done: (String) -> Void

    @State private var name: String
    @Environment(\.dismiss) private var dismiss

    init(rename: LibraryView.Rename, done: @escaping (String) -> Void) {
        self.rename = rename
        self.done = done
        _name = State(initialValue: rename.name)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Rename Group").font(.headline)
            TextField("Name", text: $name)
                .textFieldStyle(.roundedBorder)
                .frame(width: 260)
                .onSubmit(save)
            Text("The folder is renamed too, so the change shows up in Finder.")
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Rename", action: save)
                    .keyboardShortcut(.defaultAction)
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding(20)
    }

    private func save() {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return }
        done(trimmed)
        dismiss()
    }
}
