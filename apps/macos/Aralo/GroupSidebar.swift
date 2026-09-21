import AppKit
import AraloKit
import SwiftUI

/// The group tree. A group lists what is inside it as well as its own
/// snippets, so the root is the whole library and there is no special case for
/// "All Snippets".
struct GroupSidebar: View {
    let store: LibraryStore
    @Binding var selection: String?
    let rename: ([String]) -> Void
    let delete: ([String]) -> Void

    var body: some View {
        List(selection: $selection) {
            if let root = store.root {
                OutlineGroup(root, children: \.subgroups) { node in
                    GroupRow(node: node)
                        .tag(node.id)
                        .contextMenu { menu(for: node) }
                }
            }
        }
        .safeAreaInset(edge: .bottom) { newGroupBar }
        .toolbar {
            ToolbarItem {
                Button {
                    store.attempt { try $0.newGroup(under: $0.selectedGroup) }
                } label: {
                    Label("New Group", systemImage: "folder.badge.plus")
                }
                .help("New group inside the selected one")
            }
        }
    }

    private var newGroupBar: some View {
        HStack {
            if let problem = store.problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .lineLimit(2)
                    .help(problem)
            }
            Spacer()
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
    }

    @ViewBuilder
    private func menu(for node: GroupNode) -> some View {
        Button("New Group Inside") {
            store.attempt { try $0.newGroup(under: node.path) }
        }
        if !node.isRoot {
            Button("Rename\u{2026}") { rename(node.path) }
        }
        Button(node.enabled ? "Turn Off" : "Turn On") {
            store.attempt { try $0.setEnabled(!node.enabled, group: node.path) }
        }
        .disabled(node.isRoot)
        Divider()
        Menu("Colour") {
            Button("None") { store.attempt { try $0.setAppearance(group: node.path, colour: nil, icon: node.icon) } }
            ForEach(GroupPalette.colours, id: \.hex) { swatch in
                Button(swatch.name) {
                    store.attempt {
                        try $0.setAppearance(group: node.path, colour: swatch.hex, icon: node.icon)
                    }
                }
            }
        }
        .disabled(node.isRoot)
        Menu("Icon") {
            Button("None") { store.attempt { try $0.setAppearance(group: node.path, colour: node.colour, icon: nil) } }
            ForEach(GroupPalette.icons, id: \.self) { symbol in
                Button {
                    store.attempt {
                        try $0.setAppearance(group: node.path, colour: node.colour, icon: symbol)
                    }
                } label: {
                    Label(symbol.replacingOccurrences(of: ".", with: " "), systemImage: symbol)
                }
            }
        }
        .disabled(node.isRoot)
        if !node.isRoot {
            Divider()
            Button("Delete\u{2026}", role: .destructive) { delete(node.path) }
        }
    }
}

private struct GroupRow: View {
    let node: GroupNode

    var body: some View {
        Label {
            Text(node.isRoot ? "All Snippets" : node.name)
            Spacer()
            Text("\(node.total)")
                .font(.caption)
                .foregroundStyle(.secondary)
                .monospacedDigit()
        } icon: {
            Image(systemName: node.icon ?? (node.isRoot ? "tray.full" : "folder"))
                .foregroundStyle(GroupPalette.colour(node.colour) ?? .accentColor)
        }
        .opacity(node.enabled ? 1 : 0.45)
        .help(node.enabled ? "" : "This group is switched off: nothing in it expands.")
    }
}

/// The colours and icons a group can be given. The file takes any string; this
/// is the short list the menu offers so that nobody has to type a hex code.
enum GroupPalette {
    struct Swatch {
        let name: String
        let hex: String
    }

    static let colours: [Swatch] = [
        Swatch(name: "Blue", hex: "#3478F6"),
        Swatch(name: "Green", hex: "#34C759"),
        Swatch(name: "Yellow", hex: "#FFCC00"),
        Swatch(name: "Orange", hex: "#FF9500"),
        Swatch(name: "Red", hex: "#FF3B30"),
        Swatch(name: "Purple", hex: "#AF52DE"),
        Swatch(name: "Grey", hex: "#8E8E93")
    ]

    static let icons = [
        "folder", "briefcase", "envelope", "chevron.left.forwardslash.chevron.right",
        "person.2", "lifepreserver", "star", "tag", "bolt"
    ]

    /// A `#RRGGBB` string as a colour. Anything else is nil: the file format
    /// stores what was written without checking it, so a group someone typed
    /// by hand falls back to the accent colour rather than failing to draw.
    static func colour(_ hex: String?) -> Color? {
        guard var text = hex else { return nil }
        if text.hasPrefix("#") { text.removeFirst() }
        guard text.count == 6, let value = UInt32(text, radix: 16) else { return nil }
        return Color(
            red: Double((value >> 16) & 0xFF) / 255,
            green: Double((value >> 8) & 0xFF) / 255,
            blue: Double(value & 0xFF) / 255
        )
    }
}
