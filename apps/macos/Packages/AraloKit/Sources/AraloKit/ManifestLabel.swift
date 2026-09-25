import SwiftUI

/// The manifest in a panel: the one-line summary of what a request sent and
/// to whom, and on a click the whole list, the kinds that had nothing to send
/// included (plan 4.10).
public struct ManifestLabel: View {
    private let manifest: ContextManifest
    @State private var isShowing = false

    public init(_ manifest: ContextManifest) {
        self.manifest = manifest
    }

    public var body: some View {
        if manifest.hasGoneOut {
            Button {
                isShowing.toggle()
            } label: {
                HStack(spacing: 4) {
                    Text(manifest.summary).lineLimit(1)
                    Image(systemName: "info.circle")
                }
            }
            .buttonStyle(.plain)
            .font(.caption)
            .foregroundStyle(.secondary)
            .help("What was sent, and to whom")
            .popover(isPresented: $isShowing, arrowEdge: .top) {
                ManifestList(manifest: manifest)
            }
        }
    }
}

/// Every line of a manifest, for the popover.
struct ManifestList: View {
    let manifest: ContextManifest

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Sent to \(manifest.destination)").font(.headline)
            if manifest.lines.isEmpty {
                Text("No context: the request declared none.").foregroundStyle(.secondary)
            } else {
                Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 4) {
                    ForEach(manifest.lines) { line in
                        GridRow {
                            Text(line.name)
                            Text(line.detail)
                                .foregroundStyle(line.bytes == nil ? .secondary : .primary)
                                .monospacedDigit()
                        }
                    }
                }
            }
            Text(manifest.closing).font(.callout).foregroundStyle(.secondary)
        }
        .padding(14)
        .frame(minWidth: 260, alignment: .leading)
    }
}
