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

    func testTheSceneAndDelegateDeliverOneLaunchIntent() {
        let inbox = PushOpenInbox()
        let info: [AnyHashable: Any] = ["room": room, "device_id": "mini", "pane_id": "w1:p1"]
        XCTAssertTrue(inbox.offer(info, notificationID: "first"))
        XCTAssertNotNil(inbox.take())
        XCTAssertFalse(inbox.offer(info, notificationID: "first"))
        XCTAssertNil(inbox.take(), "the delegate must not reopen the detail after the scene delivered it")
        XCTAssertTrue(inbox.offer(info, notificationID: "second"))
        XCTAssertNotNil(inbox.take(), "a later notification of the same agent still opens it")
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

    /// The Reply action is the only one, it asks for Face ID or the passcode first, and it takes text; a notification that is
    /// not a question has no action.
    func testTheCategoriesOfferOnlyAReplyThatNeedsAnUnlock() throws {
        let categories = Dictionary(uniqueKeysWithValues: PushCategories.all().map { ($0.identifier, $0) })
        XCTAssertEqual(Set(categories.keys), [NoticeCategory.reply, NoticeCategory.open])
        XCTAssertTrue(try XCTUnwrap(categories[NoticeCategory.open]).actions.isEmpty)
        let actions = try XCTUnwrap(categories[NoticeCategory.reply]).actions
        XCTAssertEqual(actions.map(\.identifier), [PushCategories.replyAction])
        XCTAssertTrue(actions[0].options.contains(.authenticationRequired))
        XCTAssertFalse(actions[0].options.contains(.foreground), "the app does not open")
        XCTAssertTrue(actions[0] is UNTextInputNotificationAction)
    }
}
