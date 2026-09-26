import AppKit
import AraloBridge
import AraloKit
import SwiftUI

/// The Library tab of Settings: where the library is, and moving it. What a
/// folder is and whether a move is safe is `LibraryLocationStore`'s, and the
/// core's; what lives here is how it looks and the folder picker.
struct LibrarySettingsView: View {
    @Bindable var store: LibraryLocationStore

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Library").font(.headline)
            HStack {
                Image(systemName: "folder")
                Text(store.location.path).textSelection(.enabled).lineLimit(2).truncationMode(.middle)
            }
            Text(syncedBy).font(.callout).foregroundStyle(.secondary)
            HStack {
                Button("Show in Finder") {
                    NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: store.location.path)])
                }
                Button("Move or Choose\u{2026}", action: pick).disabled(store.isMoving)
                if store.isMoving { ProgressView().controlSize(.small) }
            }
            Text(
                "Pick an empty folder to move the library there, in iCloud Drive or Dropbox to sync it "
                    + "between Macs. Pick a library another Mac already synced to use it as it is."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            if let outcome = store.outcome {
                Text(outcome).font(.callout).fixedSize(horizontal: false, vertical: true)
            }
            if store.leftBehind != nil {
                Button("Move Old Folder to Trash") { store.trashLeftBehind() }
            }
            if let failure = store.failure {
                Label(failure, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer()
        }
        .padding(20)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var syncedBy: String {
        guard let provider = store.location.provider else {
            return "On this Mac only. Aralo does not sync it; a folder a sync client looks after does."
        }
        return "Synced by \(provider). Other Macs see changes when \(provider) delivers them."
    }

    private func pick() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        panel.message = "Choose an empty folder to move the library to, or a library to use."
        guard panel.runModal() == .OK, let folder = panel.url else { return }
        Task { await store.choose(folder) }
    }
}
