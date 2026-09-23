import AraloBridge
import AraloKit
import SwiftUI

/// The sheet an import happens in: what the file would become, the choices
/// that change that, and then the report of what it became. Everything it does
/// is a call on `LibraryImport`; what lives here is how it looks.
struct ImportSheet: View {
    @Bindable var job: LibraryImport
    /// Opens a snippet the import wrote, and closes the sheet.
    let open: (ImportedSnippet) -> Void

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            header
            if !job.isDone { options }
            report
            if let failure = job.failure {
                Label(failure, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
            buttons
        }
        .padding(20)
        .frame(minWidth: 620, idealWidth: 680, minHeight: 480, idealHeight: 560)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(job.isDone ? "Imported" : "Import \u{201C}\(job.sourceName)\u{201D}").font(.headline)
            if job.summary != nil {
                Text(job.headline)
                Text(job.detail)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var options: some View {
        Form {
            Picker("File format", selection: $job.format) {
                ForEach(LibraryImport.Format.allCases, id: \.self) { Text($0.title).tag($0) }
            }
            Picker("Macros in the text", selection: $job.macros) {
                ForEach(MacroHandling.allCases, id: \.self) { Text($0.title).tag($0) }
            }
            TextField("Into group", text: $job.group, prompt: Text("The file\u{2019}s own groups"))
            Text(job.destination)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .formStyle(.columns)
    }

    @ViewBuilder
    private var report: some View {
        if job.summary == nil {
            ContentUnavailableView(
                "Nothing to Import",
                systemImage: "doc.questionmark",
                description: Text("Aralo reads TextExpander exports, and CSV, JSON and YAML files.")
            )
        } else {
            List {
                ForEach(job.sections) { section in
                    Section {
                        ForEach(Array(section.entries.enumerated()), id: \.offset) { _, entry in
                            row(entry)
                        }
                    } header: {
                        Text("\(section.title) (\(section.entries.count))")
                    }
                }
            }
            .listStyle(.inset(alternatesRowBackgrounds: true))
        }
    }

    private func row(_ entry: ImportedSnippet) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Image(systemName: symbol(entry.outcome))
                .foregroundStyle(colour(entry.outcome))
                .accessibilityLabel(accessibility(entry.outcome))
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(entry.label.isEmpty ? "Untitled" : entry.label)
                    if !entry.abbreviations.isEmpty {
                        Text(entry.abbreviations.joined(separator: "  "))
                            .font(.body.monospaced())
                            .foregroundStyle(.secondary)
                    }
                }
                ForEach(entry.notes, id: \.self) { note in
                    Text(note).font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer()
            if job.isDone, entry.outcome == .needsEdit, entry.id != nil {
                Button("Edit") {
                    open(entry)
                    dismiss()
                }
                .help("Open this snippet to fix what did not convert")
            }
        }
    }

    private var buttons: some View {
        HStack {
            if job.isDone {
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            } else {
                Button("Cancel", role: .cancel) { dismiss() }
                Spacer()
                Button(importTitle) { job.run() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!job.canImport)
            }
        }
    }

    private var importTitle: String {
        guard let imported = job.summary?.imported, imported > 0 else { return "Import" }
        return imported == 1 ? "Import 1 Snippet" : "Import \(imported) Snippets"
    }

    private func symbol(_ outcome: ImportOutcome) -> String {
        switch outcome {
        case .clean: "checkmark.circle.fill"
        case .needsEdit: "pencil.circle.fill"
        case .skipped: "minus.circle.fill"
        }
    }

    private func colour(_ outcome: ImportOutcome) -> Color {
        switch outcome {
        case .clean: .green
        case .needsEdit: .orange
        case .skipped: .secondary
        }
    }

    private func accessibility(_ outcome: ImportOutcome) -> String {
        switch outcome {
        case .clean: "Imported as it was"
        case .needsEdit: "Needs an edit"
        case .skipped: "Not imported"
        }
    }
}
