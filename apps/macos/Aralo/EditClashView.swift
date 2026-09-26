import AraloKit
import SwiftUI

/// The sheet a save opens when the snippet's file changed on disk where the
/// draft changed it: the two versions side by side and the ways out. Nothing
/// has been written when it opens. Everything it does is a call on
/// `LibraryStore`; what lives here is how it looks. It closes when the store's
/// clash does, so a save that finds a second clash shows that one instead.
struct EditClashView: View {
    let store: LibraryStore
    let clash: EditClash

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            header
            HStack(alignment: .top, spacing: 12) {
                side("In This Window", text: clash.mineText)
                side("On Disk Now", text: clash.diskText)
            }
            DisclosureGroup("As it was when you opened it") {
                ClashText(text: clash.baseText).frame(height: 120)
            }
            .font(.callout)
            buttons
        }
        .padding(20)
        .frame(minWidth: 760, minHeight: 440)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("\u{201C}\(clash.name)\u{201D} Changed While It Was Open").font(.headline)
            Text(
                "Its file was changed on disk, by another Mac through sync or by another editor, "
                    + "after you started editing it here. \(clash.summary)"
            )
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Text("Nothing has been saved yet.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private func side(_ title: String, text: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.subheadline.bold())
            ClashText(text: text)
        }
        .frame(maxWidth: .infinity)
    }

    private var buttons: some View {
        HStack {
            Button("Cancel", role: .cancel) { dismiss() }
            Spacer()
            Button("Use the File on Disk") {
                store.attempt { try $0.useTheirs() }
            }
            .disabled(!clash.diskIsReadable)
            .help("Drop what you typed and show the file as it is now")
            Button("Edit Mine First") {
                store.editBeforeSaving()
            }
            .disabled(!clash.diskIsReadable)
            .help("Keep what you typed on screen, unsaved, to fold in what you want from the file")
            Button("Save Mine") {
                store.attempt { try $0.keepMine() }
            }
            .keyboardShortcut(.defaultAction)
            .help("Save what you typed. Anything else changed on disk stays.")
        }
    }
}

/// A file's text, read-only, scrolling and selectable.
private struct ClashText: View {
    let text: String

    var body: some View {
        ScrollView([.vertical, .horizontal]) {
            Text(text)
                .font(.body.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .padding(8)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 6))
    }
}
