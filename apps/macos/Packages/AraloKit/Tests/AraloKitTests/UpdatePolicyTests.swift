import AraloKit
import XCTest

final class UpdatePolicyTests: XCTestCase {
    private let feedURL = "https://github.com/anandghegde/aralo/releases/latest/download/appcast.xml"
    private let key = Data(repeating: 7, count: 32).base64EncodedString()

    func testTheProjectPlaceholdersAreNotAFeed() {
        // What apps/macos/project.yml puts in a local build.
        let local = UpdateFeed(infoDictionary: [
            "SUFeedURL": feedURL,
            "SUPublicEDKey": "PLACEHOLDER-set-by-scripts-release.sh"
        ])
        XCTAssertFalse(local.isConfigured)
        XCTAssertFalse(UpdatePolicy.mayCheck(feed: local, localOnly: false))
        XCTAssertFalse(UpdateFeed(infoDictionary: [:]).isConfigured)
        XCTAssertFalse(UpdateFeed(infoDictionary: ["SUFeedURL": "", "SUPublicEDKey": ""]).isConfigured)
    }

    func testAReleaseFeedIsChecked() {
        let feed = UpdateFeed(infoDictionary: ["SUFeedURL": feedURL, "SUPublicEDKey": key])
        XCTAssertTrue(feed.isConfigured)
        XCTAssertTrue(UpdatePolicy.mayCheck(feed: feed, localOnly: false))
    }

    /// PRD P6: local-only mode means no network, the update check included.
    func testLocalOnlyModeTurnsTheUpdateCheckOff() {
        let feed = UpdateFeed(infoDictionary: ["SUFeedURL": feedURL, "SUPublicEDKey": key])
        XCTAssertFalse(UpdatePolicy.mayCheck(feed: feed, localOnly: true))
    }

    /// data-flow.md lists github.com for updates, over https, and nothing else.
    func testOnlyAnHttpsFeedOnGitHubCounts() {
        for address in [
            "http://github.com/anandghegde/aralo/releases/latest/download/appcast.xml",
            "https://example.com/appcast.xml",
            "https://github.com.example.com/appcast.xml"
        ] {
            let feed = UpdateFeed(infoDictionary: ["SUFeedURL": address, "SUPublicEDKey": key])
            XCTAssertFalse(feed.isConfigured, address)
        }
    }

    func testTheKeyMustBeThirtyTwoBytes() {
        let short = Data(repeating: 7, count: 31).base64EncodedString()
        let feed = UpdateFeed(infoDictionary: ["SUFeedURL": feedURL, "SUPublicEDKey": short])
        XCTAssertFalse(feed.isConfigured)
    }
}
