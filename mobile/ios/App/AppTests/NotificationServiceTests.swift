import UserNotifications
import XCTest
import HidePushCore

/// The notification extension's own class (`HideNotification/NotificationService.swift`, compiled into this bundle too),
/// run in the app on the simulator against the real Keychain group it reads: what it makes of a notice it can open, and
/// that it leaves the fixed text in every other case. `simctl push` does not run service extensions, so this is where the
/// class is exercised on a simulator; the real delivery is only provable on an iPhone.
final class NotificationServiceTests: XCTestCase {
    private let room = String(repeating: "n", count: 43)
    private let other = String(repeating: "o", count: 43)
    private var keychain: PushKeychain!

    override func setUpWithError() throws {
        keychain = PushKeychain(accessGroup: PushKeychain.sharedAccessGroup())
        // The extension reads the default service; these tests use it with rooms nothing else does.
        try keychain.remove(room: room)
        try keychain.remove(room: other)
    }

    override func tearDownWithError() throws {
        try keychain.remove(room: room)
        try keychain.remove(room: other)
    }

    private func deliver(userInfo: [AnyHashable: Any]) throws -> UNNotificationContent {
        let content = UNMutableNotificationContent()
        content.body = "Hide has news"
        content.userInfo = userInfo
        let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        let done = expectation(description: "content handler")
        var result: UNNotificationContent?
        NotificationService().didReceive(request) { result = $0; done.fulfill() }
        wait(for: [done], timeout: 5)
        return try XCTUnwrap(result)
    }

    func testAQuestionForAHeldKeyBecomesTheNoticeWithAReplyAndANamedAgent() throws {
        let key = try keychain.create(room: room)
        let hide = try NoticeSealer.seal(NoticeSealer.notice(room: room), to: key)
        let out = try deliver(userInfo: ["hide": hide])
        XCTAssertEqual(out.title, "Fix the build")
        XCTAssertTrue(out.body.contains("Should I update the lockfile too?"))
        XCTAssertEqual(out.categoryIdentifier, NoticeCategory.reply)
        XCTAssertEqual(out.userInfo[NoticeInfo.room] as? String, room)
        XCTAssertEqual(out.userInfo[NoticeInfo.deviceId] as? String, "dev")
        XCTAssertEqual(out.userInfo[NoticeInfo.paneId] as? String, "w1:p2")
        XCTAssertEqual(out.userInfo[NoticeInfo.turn] as? String, "1794300000000")
        XCTAssertNil(out.userInfo["hide"], "the ciphertext is not kept")
    }

    func testAnApprovalOffersNoActionAndTheTapOpensTheAgent() throws {
        let key = try keychain.create(room: room)
        let hide = try NoticeSealer.seal(NoticeSealer.notice(room: room, kind: "approval"), to: key)
        let out = try deliver(userInfo: ["hide": hide])
        XCTAssertEqual(out.categoryIdentifier, NoticeCategory.open)
        XCTAssertEqual(out.userInfo[NoticeInfo.paneId] as? String, "w1:p2")
    }

    func testANoticeForAnotherComputerKeepsTheFixedTextAndNamesNothing() throws {
        let key = try keychain.create(room: room)
        let hide = try NoticeSealer.seal(NoticeSealer.notice(room: other), to: key)
        let out = try deliver(userInfo: ["hide": hide])
        XCTAssertEqual(out.body, "Hide has news")
        XCTAssertEqual(out.title, "")
        XCTAssertTrue(out.userInfo.isEmpty)
        XCTAssertEqual(out.categoryIdentifier, "")
    }

    func testABodyNoKeyOpensKeepsTheFixedTextAndForgetsTheCiphertext() throws {
        let made = try keychain.create(room: other)
        try keychain.remove(room: other)
        let hide = try NoticeSealer.seal(NoticeSealer.notice(room: other), to: made)
        let out = try deliver(userInfo: ["hide": hide])
        XCTAssertEqual(out.body, "Hide has news")
        XCTAssertTrue(out.userInfo.isEmpty)
    }

    func testAPushWithNoBodyKeepsTheFixedText() throws {
        _ = try keychain.create(room: room)
        let out = try deliver(userInfo: [:])
        XCTAssertEqual(out.body, "Hide has news")
        XCTAssertTrue(out.userInfo.isEmpty)
    }
}
