import UserNotifications
import XCTest
import HidePushCore
@testable import HideShell

final class PushTests: XCTestCase {
    let room = String(repeating: "A", count: 43)

    func testATapOpensTheAgentOnlyForTheShapesTheExtensionWrites() {
        let inbox = PushOpenInbox()
        XCTAssertFalse(inbox.offer([:]), "the fixed text opens no agent")
        XCTAssertFalse(inbox.offer(["room": "short", "device_id": "mini", "pane_id": "w1:p1"]))
        XCTAssertFalse(inbox.offer(["room": room, "device_id": "", "pane_id": "w1:p1"]))
        XCTAssertNil(inbox.take())
        XCTAssertTrue(inbox.offer(["room": room, "device_id": "mini", "pane_id": "w1:p1"]))
        XCTAssertEqual(inbox.take(), PushOpenInbox.Target(room: room, deviceId: "mini", paneId: "w1:p1"))
        XCTAssertNil(inbox.take(), "given out once")
    }

    func testTheTokenIsAskedForOnceAndAnswersEveryWaiter() {
        let token = PushToken()
        var registered = 0
        token.register = { registered += 1 }
        var answers: [Result<String, PushToken.Failure>] = []
        token.request { answers.append($0) }
        token.request { answers.append($0) }
        XCTAssertEqual(registered, 1, "one registration in flight")
        token.didRegister(deviceToken: Data([0xAB, 0x01, 0xFF]))
        XCTAssertEqual(answers.count, 2)
        XCTAssertEqual(try answers[0].get(), "ab01ff")
        XCTAssertEqual(try answers[1].get(), "ab01ff")
        var failed: Result<String, PushToken.Failure>?
        token.request { failed = $0 }
        token.didFail()
        XCTAssertEqual(failed, .failure(.unavailable))
    }

    func testTheTokenGivesUpWhenAppleDoesNotAnswer() {
        let token = PushToken()
        token.register = {}
        token.wait = 0.05
        let done = expectation(description: "timed out")
        token.request { result in
            XCTAssertEqual(result, .failure(.timedOut))
            done.fulfill()
        }
        wait(for: [done], timeout: 5)
    }

    func testAnEarlierRequestsTimerDoesNotEndALaterRequest() {
        func pause(_ seconds: TimeInterval) {
            let gap = expectation(description: "pause")
            DispatchQueue.global().asyncAfter(deadline: .now() + seconds) { gap.fulfill() }
            wait(for: [gap], timeout: 5)
        }
        let token = PushToken()
        token.register = {}
        token.wait = 0.3
        var first: Result<String, PushToken.Failure>?
        token.request { first = $0 }
        token.didFail()
        XCTAssertEqual(first, .failure(.unavailable))
        // The second round starts at 0.2 s and its own timer ends at 0.5 s; the first round's timer fires at 0.3 s.
        pause(0.2)
        var second: Result<String, PushToken.Failure>?
        token.request { second = $0 }
        pause(0.2)
        XCTAssertNil(second, "the first round's timer ended the second round")
        pause(0.3)
        XCTAssertEqual(second, .failure(.timedOut))
    }

    func testTheAppStopsReceivingNoticesOnlyWhenNoComputersKeyIsLeft() {
        let token = PushToken()
        var unregistered = 0
        token.unregister = { unregistered += 1 }
        token.keysChanged(remaining: 2)
        token.keysChanged(remaining: 1)
        XCTAssertEqual(unregistered, 0)
        token.keysChanged(remaining: 0)
        XCTAssertEqual(unregistered, 1)
    }

    func testTheEnvironmentIsTheBuildsAndNeverADefault() {
        #if DEBUG
        XCTAssertEqual(PushEnvironment.current, "sandbox")
        #else
        XCTAssertEqual(PushEnvironment.current, "production")
        #endif
    }

    func testNewAndAlreadyDeliveredNotificationsOfferNoActions() throws {
        let categories = Dictionary(uniqueKeysWithValues: PushCategories.all().map { ($0.identifier, $0) })
        XCTAssertEqual(Set(categories.keys), [NoticeCategory.open, "hide.reply"])
        for category in categories.values {
            XCTAssertTrue(category.actions.isEmpty, "a notification opens its conversation; it offers no Reply action")
        }
    }
}
