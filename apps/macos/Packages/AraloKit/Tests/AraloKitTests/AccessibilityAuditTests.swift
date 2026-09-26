import AppKit
@testable import AraloKit
import SwiftUI
import XCTest

/// The audit itself: that it finds what VoiceOver would read out by role
/// alone, and passes what it would read out by name. The app's own panels
/// are audited in `AraloTests`, where their views live.
@MainActor
final class AccessibilityAuditTests: XCTestCase {
    /// A view in a window, the way VoiceOver meets it.
    private func audit(_ view: some View) -> [AccessibilityAudit.Finding] {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 400, height: 300), styleMask: [.titled],
            backing: .buffered, defer: false
        )
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: view.frame(width: 400, height: 300))
        defer { window.close() }
        return AccessibilityAudit.unlabelled(in: window)
    }

    func testWhatVoiceOverCouldOnlyReadByRoleIsFound() {
        let findings = audit(
            VStack {
                // SwiftUI has no word for this symbol, so it reads its name.
                Button {} label: { Image(systemName: "exclamationmark.arrow.triangle.2.circlepath") }
                TextEditor(text: .constant(""))
                Picker("", selection: .constant(1)) { Text("One").tag(1) }.labelsHidden()
            }
        )
        XCTAssertEqual(findings.map(\.role), ["AXButton", "AXTextArea", "AXPopUpButton"], "\(findings)")
        XCTAssertEqual(findings.first?.symbol, "exclamationmark.arrow.triangle.2.circlepath")
    }

    func testNamedControlsPass() {
        let findings = audit(
            VStack {
                Button {} label: { Image(systemName: "exclamationmark.arrow.triangle.2.circlepath") }
                    .accessibilityLabel("Sync Conflicts")
                Button("Continue") {}
                TextField("Name", text: .constant(""))
                TextEditor(text: .constant("")).accessibilityLabel("Headers")
                Picker("Model", selection: .constant(1)) { Text("One").tag(1) }.labelsHidden()
                Toggle("Use AI", isOn: .constant(false))
                Image(systemName: "exclamationmark.triangle").accessibilityHidden(true)
                Label("Granted", systemImage: "checkmark.circle.fill")
            }
        )
        XCTAssertEqual(findings, [], "\(findings)")
    }

    func testAFindingSaysWhereItIs() {
        let findings = audit(
            VStack {
                TextEditor(text: .constant(""))
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Profiles")
        )
        XCTAssertEqual(findings.count, 1, "\(findings)")
        XCTAssertEqual(findings.first?.path.last, "Profiles", "\(findings)")
        XCTAssertTrue(findings.first?.description.hasSuffix("AXTextArea") ?? false, "\(findings)")
    }

    /// A form row names its control with the text beside it, and VoiceOver
    /// reads that text as the control's name.
    func testAControlNamedByItsFormRowPasses() {
        let findings = audit(
            Form {
                TextField("Name", text: .constant("Local"))
                Picker("Model", selection: .constant(1)) { Text("One").tag(1) }
            }
            .formStyle(.columns)
        )
        XCTAssertEqual(findings, [], "\(findings)")
    }

    func testTheManifestLabelIsNamed() {
        let manifest = ContextManifest([], profile: "Local", model: "llama")
        let findings = audit(ManifestLabel(manifest))
        XCTAssertEqual(findings, [], "\(findings)")
    }

    func testSymbolNamesAreToldFromWords() {
        XCTAssertTrue(AccessibilityAudit.isSymbolName("text.magnifyingglass"))
        XCTAssertFalse(AccessibilityAudit.isSymbolName("Remove"))
        XCTAssertFalse(AccessibilityAudit.isSymbolName("Wait for it."))
    }
}
