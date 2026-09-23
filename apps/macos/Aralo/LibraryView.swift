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
    /// Asks for a file to import. The panel is the window's, so it is the
    /// controller's to show.
    var chooseImport: () -> Void = {}
    /// Asks where to write the selected group, in the format given.
    var export: (ExportFormat) -> Void = { _ in }

    @State private var renaming: Rename?
    @State private var deletingGroup: [String]?
    @State private var resolving: ConflictResolver?

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
        .toolbar {
            conflictsButton
            interchangeMenu
        }
        .sheet(item: $resolving) { resolver in
            ConflictResolverView(resolver: resolver)
        }
        .sheet(item: $store.importing) { job in
            ImportSheet(job: job) { entry in store.open(imported: entry) }
        }
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
            SnippetEditorView(store: store, editing: editing, root: root, resolve: resolve)
        } else {
            ContentUnavailableView(
                "No Snippet Selected",
                systemImage: "text.cursor",
                description: Text("Pick one from the list, or press \u{2318}N to write a new one.")
            )
        }
    }

    // MARK: - Import and export

    /// Import brings snippets in; export writes out the group the sidebar has
    /// selected, which at the root is the whole library.
    @ToolbarContentBuilder
    private var interchangeMenu: some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            Menu {
                Button("Import Snippets\u{2026}", action: chooseImport)
                Divider()
                Section(exportTitle) {
                    ForEach(ExportFormat.allCases, id: \.self) { format in
                        Button("\(format.title)\u{2026}") {
                            leave()
                            export(format)
                        }
                    }
                }
            } label: {
                Label("Import and Export", systemImage: "square.and.arrow.down.on.square")
            }
            .help("Bring snippets in from another expander, or write these out")
        }
    }

    private var exportTitle: String {
        store.selectedGroup.isEmpty
            ? "Export the Library As"
            : "Export \u{201C}\(store.selectedGroup.last ?? "")\u{201D} As"
    }

    // MARK: - Sync conflicts

    /// Shown only while a conflict copy waits: one per copy, named after the
    /// snippet it is a copy of.
    @ToolbarContentBuilder
    private var conflictsButton: some ToolbarContent {
        if !store.conflicts.isEmpty {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    ForEach(store.conflicts, id: \.copy) { conflict in
                        Button(conflict.original) { resolve(conflict.copy) }
                    }
                } label: {
                    Label(conflictsTitle, systemImage: "exclamationmark.arrow.triangle.2.circlepath")
                }
                .help("Sync made two versions of a snippet. Choose which to keep.")
            }
        }
    }

    private var conflictsTitle: String {
        store.conflicts.count == 1 ? "1 Sync Conflict" : "\(store.conflicts.count) Sync Conflicts"
    }

    /// Saves what is typed first, so the resolver shows the file as it now
    /// is and the user's edit is one of the versions to choose from.
    private func resolve(_ copy: String) {
        leave()
        resolving = store.resolver(for: copy)
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
