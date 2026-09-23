import AraloKit
import SwiftUI

/// The sheet a sync conflict opens in: this Mac's version and the copy side by
/// side, what they changed differently, and the three ways out. Everything it
/// does is a call on `ConflictResolver`; what lives here is how it looks.
struct ConflictResolverView: View {
    @Bindable var resolver: ConflictResolver

    @State private var writing = false
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            header
            if writing {
                editor
            } else {
                sides
            }
            if let failure = resolver.failure {
                Label(failure, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                    .font(.callout)
            }
            buttons
        }
        .padding(20)
        .frame(minWidth: 760, minHeight: 440)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Two Versions of \u{201C}\(resolver.originalName)\u{201D}").font(.headline)
            Text(
                "A sync client kept both, because this Mac and another one changed the snippet "
                    + "before either saw the other\u{2019}s change. \(resolver.summary)"
            )
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Text("Whichever you keep goes into \(resolver.originalName). The copy goes to the Trash.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var sides: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 12) {
                side("This Mac", file: resolver.originalName, text: resolver.detail.originalText)
                side("The Copy", file: resolver.copyName, text: resolver.detail.copyText)
            }
            if let base = resolver.detail.baseText {
                DisclosureGroup("The version both started from") {
                    FileText(text: base).frame(height: 120)
                }
                .font(.callout)
            }
        }
    }

    private func side(_ title: String, file: String, text: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.subheadline.bold())
            Text(file).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
            FileText(text: text)
        }
        .frame(maxWidth: .infinity)
    }

    private var editor: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("The Version to Keep").font(.subheadline.bold())
            Text("The whole file, as a text editor would show it. It starts as this Mac\u{2019}s.")
                .font(.caption)
                .foregroundStyle(.secondary)
            TextEditor(text: $resolver.edited)
                .font(.body.monospaced())
                .scrollContentBackground(.hidden)
                .padding(6)
                .background(.background.secondary, in: RoundedRectangle(cornerRadius: 6))
        }
    }

    private var buttons: some View {
        HStack {
            Button("Cancel", role: .cancel) { dismiss() }
            Spacer()
            if writing {
                Button("Back") { writing = false }
                Button("Keep This Version") { finish(resolver.keepEdited()) }
                    .keyboardShortcut(.defaultAction)
            } else {
                Button("Write a Version\u{2026}") { writing = true }
                Button("Keep the Copy") { finish(resolver.keepCopy()) }
                    .disabled(!resolver.copyIsReadable)
                Button("Keep This Mac\u{2019}s") { finish(resolver.keepOriginal()) }
                    .keyboardShortcut(.defaultAction)
            }
        }
    }

    private func finish(_ resolved: Bool) {
        if resolved { dismiss() }
    }
}

/// A file's text, read-only, scrolling, and selectable so a line can be copied
/// out of either side.
private struct FileText: View {
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
