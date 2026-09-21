import AppKit
import AraloKit
import SwiftUI

/// The snippets in the selected group, narrowed by the search box.
struct SnippetList: View {
    let store: LibraryStore
    @Binding var selection: String?
    let root: URL

    var body: some View {
        List(store.rows, selection: $selection) { row in
            SnippetRowView(row: row, searching: !store.query.isEmpty)
                .tag(row.id)
                .contextMenu { menu(for: row) }
        }
        .overlay {
            if store.rows.isEmpty {
                ContentUnavailableView(
                    store.query.isEmpty ? "No Snippets Here" : "No Matches",
                    systemImage: store.query.isEmpty ? "tray" : "magnifyingglass",
                    description: Text(
                        store.query.isEmpty
                            ? "Press \u{2318}N to write one."
                            : "Nothing in this group matches \u{201C}\(store.query)\u{201D}."
                    )
                )
            }
        }
        .searchable(text: query, placement: .toolbar, prompt: "Search snippets")
        .toolbar {
            ToolbarItem {
                Button {
                    store.attempt { try $0.newSnippet() }
                } label: {
                    Label("New Snippet", systemImage: "plus")
                }
                .keyboardShortcut("n")
                .help("New snippet in the selected group")
            }
        }
    }

    private var query: Binding<String> {
        Binding(get: { store.query }, set: { store.search($0) })
    }

    @ViewBuilder
    private func menu(for row: SnippetRow) -> some View {
        Button(row.enabled ? "Turn Off" : "Turn On") {
            store.attempt { try $0.setEnabled(!row.enabled, snippet: row.id) }
        }
        Menu("Move To") {
            ForEach(store.root?.flattened ?? []) { node in
                Button(node.isRoot ? "All Snippets" : node.path.joined(separator: " \u{203A} ")) {
                    store.attempt { try $0.move(snippet: row.id, to: node.path) }
                }
                .disabled(node.path == row.group)
            }
        }
        Divider()
        Button("Delete", role: .destructive) {
            store.attempt { try $0.delete(snippet: row.id) }
        }
    }
}

private struct SnippetRowView: View {
    let row: SnippetRow
    let searching: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(row.label).lineLimit(1)
                Spacer(minLength: 4)
                if let abbreviation = row.abbreviations.first {
                    Text(abbreviation)
                        .font(.caption.monospaced())
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(.quaternary, in: RoundedRectangle(cornerRadius: 4))
                }
            }
            Text(caption)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .opacity(row.enabled ? 1 : 0.45)
        .padding(.vertical, 2)
    }

    /// What the row says underneath the label: where the search matched, or
    /// the group when nothing was searched for.
    private var caption: AttributedString {
        guard searching else {
            let group = row.group.isEmpty ? "All Snippets" : row.group.joined(separator: " \u{203A} ")
            return AttributedString(group)
        }
        var text = AttributedString(row.detail)
        for offset in row.matched where offset >= 0 && offset < row.detail.count {
            let start = text.index(text.startIndex, offsetByCharacters: offset)
            let end = text.index(start, offsetByCharacters: 1)
            text[start..<end].foregroundColor = .accentColor
            text[start..<end].inlinePresentationIntent = .stronglyEmphasized
        }
        return text
    }
}
