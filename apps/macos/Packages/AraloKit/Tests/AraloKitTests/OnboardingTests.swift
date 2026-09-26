import AraloBridge
@testable import AraloKit
import XCTest

final class OnboardingTests: XCTestCase {
    private func facts(accessibility: Bool = false, inputMonitoring: Bool = false, tapRunning: Bool = false)
        -> OnboardingFlow.Facts {
        OnboardingFlow.Facts(
            permissions: Permissions.Status(accessibility: accessibility, inputMonitoring: inputMonitoring),
            tapRunning: tapRunning
        )
    }

    func testAFreshUserWalksEveryStepAndWaitsAtEachPermission() {
        var flow = OnboardingFlow()
        XCTAssertEqual(flow.step, .privacy)
        XCTAssertTrue(flow.advance(facts()))
        XCTAssertEqual(flow.step, .accessibility)

        XCTAssertFalse(flow.canContinue(facts()))
        XCTAssertFalse(flow.advance(facts()), "not granted yet")
        XCTAssertEqual(flow.step, .accessibility)

        XCTAssertTrue(flow.advance(facts(accessibility: true)))
        XCTAssertEqual(flow.step, .inputMonitoring)
        XCTAssertFalse(flow.advance(facts(accessibility: true)))

        let granted = facts(accessibility: true, inputMonitoring: true)
        XCTAssertTrue(flow.advance(granted))
        XCTAssertEqual(flow.step, .library)
        XCTAssertEqual(flow.position, 4)
        XCTAssertTrue(flow.advance(granted))
        XCTAssertEqual(flow.step, .aiOptIn, "AI is offered, and Continue passes it by")
        XCTAssertTrue(flow.advance(granted))
        XCTAssertEqual(flow.step, .tryIt)
        XCTAssertEqual(flow.position, flow.steps.count)
        XCTAssertTrue(flow.isLast)
        XCTAssertFalse(flow.advance(granted), "the flow is over")
    }

    func testTheSetUpScreensAreNotSkippedWhenEverythingIsGranted() {
        var flow = OnboardingFlow()
        flow.advance(facts(accessibility: true, inputMonitoring: true))
        XCTAssertEqual(flow.step, .library)
        flow.back()
        XCTAssertEqual(flow.step, .inputMonitoring)
    }

    func testAReturningUserIsOnlyAskedForWhatWasRevoked() {
        var flow = OnboardingFlow(startingAt: .accessibility, returning: true)
        XCTAssertEqual(flow.steps, [.privacy, .accessibility, .inputMonitoring, .tryIt])
        XCTAssertTrue(flow.advance(facts(accessibility: true, inputMonitoring: true)))
        XCTAssertEqual(flow.step, .tryIt, "no library or AI screen the second time")
        flow.back()
        XCTAssertEqual(flow.step, .inputMonitoring)
        XCTAssertEqual(flow.position, 3)
    }

    func testTheLibraryScreenSaysWhoseSnippetsTheyAre() {
        let folder = URL(fileURLWithPath: "/Volumes/Work/Snippets")
        let made = OnboardingLibrary(snippetCount: 12, starterFiles: 5, folder: folder)
        XCTAssertTrue(made.summary.hasPrefix("Aralo made your library and put 12 snippets"), made.summary)
        let own = OnboardingLibrary(snippetCount: 1, starterFiles: 0, folder: folder)
        XCTAssertEqual(own.summary, "Your library already has 1 snippet.")
        let empty = OnboardingLibrary(snippetCount: 0, starterFiles: 0, folder: folder)
        XCTAssertTrue(empty.summary.hasPrefix("Your library is empty."), empty.summary)
        XCTAssertEqual(made.displayPath, "/Volumes/Work/Snippets")

        let home = FileManager.default.homeDirectoryForCurrentUser
        let inHome = OnboardingLibrary(
            snippetCount: 0, starterFiles: 0, folder: home.appendingPathComponent("Aralo Snippets")
        )
        XCTAssertEqual(inHome.displayPath, "~/Aralo Snippets")
    }

    func testWhatIsAlreadyGrantedIsNotAskedFor() {
        var flow = OnboardingFlow()
        flow.advance(facts(accessibility: true, inputMonitoring: true))
        XCTAssertEqual(flow.step, .library)

        flow = OnboardingFlow()
        flow.advance(facts(accessibility: true))
        XCTAssertEqual(flow.step, .inputMonitoring)
        flow.back()
        XCTAssertEqual(flow.step, .accessibility)
        flow.back()
        flow.back()
        XCTAssertEqual(flow.step, .privacy)
    }

    func testARunningTapIsWorthMoreThanAMissingInputMonitoringGrant() {
        var flow = OnboardingFlow(startingAt: .inputMonitoring)
        XCTAssertTrue(flow.canContinue(facts(accessibility: true, tapRunning: true)))
        flow = OnboardingFlow()
        flow.advance(facts(accessibility: true, tapRunning: true))
        XCTAssertEqual(flow.step, .library)
    }

    func testTheFlowOpensAtTheStartOnceAndLaterOnlyAtWhatWasRevoked() {
        let granted = facts(accessibility: true, inputMonitoring: true, tapRunning: true)
        XCTAssertEqual(OnboardingFlow.entryPoint(completedBefore: false, facts: granted), .privacy)
        XCTAssertNil(OnboardingFlow.entryPoint(completedBefore: true, facts: granted))
        XCTAssertEqual(OnboardingFlow.entryPoint(completedBefore: true, facts: facts()), .accessibility)
        XCTAssertEqual(
            OnboardingFlow.entryPoint(completedBefore: true, facts: facts(accessibility: true)), .inputMonitoring
        )
    }

    private func snippet(_ abbreviation: String?, _ preview: String, enabled: Bool = true) -> SnippetSummary {
        SnippetSummary(
            id: "", label: "", abbreviations: abbreviation.map { [$0] } ?? [], group: [], path: "",
            enabled: enabled, preview: preview
        )
    }

    func testTheExampleIsTheStarterSnippetOrTheFirstOneThatCanBeRecognised() {
        let starter = [snippet(";br", "Best regards,"), snippet("ty", "thank you")]
        XCTAssertEqual(
            OnboardingExample.pick(from: starter), OnboardingExample(abbreviation: "ty", expansion: "thank you")
        )

        let own = [
            snippet(nil, "no abbreviation"), snippet(";off", "disabled", enabled: false),
            snippet(";d", "{{date}}"), snippet(";sig", "Sam\nExample Ltd"), snippet(";hi", " hello there ")
        ]
        XCTAssertEqual(
            OnboardingExample.pick(from: own), OnboardingExample(abbreviation: ";hi", expansion: "hello there")
        )
        XCTAssertNil(OnboardingExample.pick(from: []))
    }

    func testTheFieldShowsTheExpansionInAnyCase() {
        let example = OnboardingExample(abbreviation: "ty", expansion: "thank you")
        XCTAssertFalse(example.isExpanded(in: "ty "))
        XCTAssertTrue(example.isExpanded(in: "Well, Thank you "))
    }
}
