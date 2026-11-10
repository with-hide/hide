import XCTest
@testable import HideShell

final class PairLinkTests: XCTestCase {
    private let good = "https://app.withhide.dev/#pair=eyJhIjoiYiJ9"

    func testOnlyThePairingLinkOfTheHostedPhoneSiteIsAccepted() {
        XCTAssertEqual(PairLink.accept(URL(string: good)), good)
        XCTAssertEqual(PairLink.accept(URL(string: "https://APP.withhide.dev/#pair=abc_-")), "https://APP.withhide.dev/#pair=abc_-")
        let refused = [
            "http://app.withhide.dev/#pair=abc",
            "https://app.withhide.dev.evil.test/#pair=abc",
            "https://evil.test/#pair=abc",
            "https://app.withhide.dev:8443/#pair=abc",
            "https://user@app.withhide.dev/#pair=abc",
            "https://app.withhide.dev/?x=1#pair=abc",
            "https://app.withhide.dev/p/5/#pair=abc",
            "https://app.withhide.dev/",
            "https://app.withhide.dev/#pair=",
            "https://app.withhide.dev/#open=abc",
            "https://app.withhide.dev/#pair=a b",
            "https://app.withhide.dev/#pair=abc%00",
            "https://app.withhide.dev/#pair=abc&x=1",
            "tel:+8210000000",
            "hide://pair=abc",
            "https://app.withhide.dev/#pair=" + String(repeating: "a", count: 1100),
        ]
        for text in refused { XCTAssertNil(PairLink.accept(URL(string: text)), text) }
        XCTAssertNil(PairLink.accept(nil))
    }

    func testTheInboxGivesALinkOnceAndOnlyForAMinute() {
        var clock = Date(timeIntervalSince1970: 1000)
        let inbox = PairLinkInbox(now: { clock })
        XCTAssertFalse(inbox.offer(URL(string: "https://evil.test/#pair=abc")))
        XCTAssertNil(inbox.take())
        XCTAssertTrue(inbox.offer(URL(string: good)))
        XCTAssertEqual(inbox.take(), good)
        XCTAssertNil(inbox.take())
        XCTAssertTrue(inbox.offer(URL(string: good)))
        clock = clock.addingTimeInterval(PairLinkInbox.lifetime + 1)
        XCTAssertNil(inbox.take())
    }

    func testAnOfferTellsTheWebSideWithoutTheLink() {
        let inbox = PairLinkInbox()
        let heard = expectation(description: "told")
        let token = NotificationCenter.default.addObserver(forName: .hidePairLinkArrived, object: inbox, queue: nil) { note in
            XCTAssertNil(note.userInfo)
            heard.fulfill()
        }
        defer { NotificationCenter.default.removeObserver(token) }
        XCTAssertTrue(inbox.offer(URL(string: good)))
        wait(for: [heard], timeout: 2)
    }
}
