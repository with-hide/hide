import XCTest
@testable import HidePushCore

private final class FakeNotifier: ReplyNotifier {
    var events: [String] = []
    var scheduled: [String: (body: String, after: TimeInterval)] = [:]
    var everScheduled: [String: (body: String, after: TimeInterval)] = [:]
    var posted: [String: String] = [:]
    func schedule(id: String, body: String, userInfo: [String: String], after: TimeInterval) {
        events.append("schedule \(id)")
        scheduled[id] = (body, after)
        everScheduled[id] = (body, after)
    }
    func cancel(id: String) {
        events.append("cancel \(id)")
        scheduled[id] = nil
    }
    func post(id: String, body: String, userInfo: [String: String]) {
        events.append("post \(id)")
        posted[id] = body
    }
}

private final class FakeLedger: ReplyLedger {
    var states: [String: ReplyState] = [:]
    var log: [String] = []
    func state(of requestId: String) -> ReplyState? { states[requestId] }
    func set(_ state: ReplyState, for requestId: String) {
        states[requestId] = state
        log.append(state.rawValue)
    }
}

private final class FakeHeld: HeldReplies {
    var texts: [String: String] = [:]
    func keep(text: String, room: String, deviceId: String, paneId: String) { texts[room + deviceId + paneId] = text }
    func take(room: String, deviceId: String, paneId: String) -> String? { texts[room + deviceId + paneId] }
    func drop(room: String, deviceId: String, paneId: String) { texts[room + deviceId + paneId] = nil }
}

private struct FakePairing: PairingSource {
    var result: Result<(noiseKey: Data, credential: String)?, Error>
    func pairing(room: String) throws -> (noiseKey: Data, credential: String)? { try result.get() }
}

private struct Locked: Error {}

/// A link whose answer the test decides. `hang` waits until `close` (the deadline's way to end a wait).
private final class FakeLink: PhoneLink {
    enum Behaviour { case answer([String: Any]), connectFails, requestFails(LinkFailure), hang }
    var behaviour: Behaviour
    var sent: [[String: Any]] = []
    var order: [String] = []
    var closed = false
    private var waiting: CheckedContinuation<Void, Never>?

    init(_ behaviour: Behaviour) { self.behaviour = behaviour }

    func connect(room: String, noiseKey: Data, credential: String) async throws {
        order.append("connect")
        if case .connectFails = behaviour { throw LinkFailure.notSent }
    }

    func request(_ message: [String: Any], resultType: String, requestId: String) async throws -> [String: Any] {
        order.append("request")
        sent.append(message)
        switch behaviour {
        case .answer(let answer): return answer
        case .requestFails(let failure): throw failure
        case .hang:
            await withCheckedContinuation { waiting = $0 }
            throw LinkFailure.unconfirmed
        case .connectFails: throw LinkFailure.notSent
        }
    }

    func close() {
        closed = true
        waiting?.resume()
        waiting = nil
    }
}

final class ReplySenderTests: XCTestCase {
    private let room = String(repeating: "r", count: 43)
    private let words = ReplyWords(stale: "stale", notSent: "not-sent", outdated: "outdated", unconfirmed: "unconfirmed")
    private var notifier = FakeNotifier()
    private var ledger = FakeLedger()
    private var held = FakeHeld()
    private var links: [FakeLink] = []

    override func setUp() {
        notifier = FakeNotifier()
        ledger = FakeLedger()
        held = FakeHeld()
        links = []
    }

    private func target(_ notification: String = "n1") -> ReplyTarget {
        ReplyTarget(userInfo: ["room": room, "device_id": "dev", "pane_id": "pane", "turn": "1700000000123"], notificationId: notification)!
    }

    private func sender(_ behaviours: [FakeLink.Behaviour], pairing: Result<(noiseKey: Data, credential: String)?, Error> = .success((Data(count: 32), "secret")),
                        deadline: TimeInterval = 25) -> ReplySender {
        var queue = behaviours
        return ReplySender(pairing: FakePairing(result: pairing), makeLink: {
            let link = FakeLink(queue.isEmpty ? .hang : queue.removeFirst())
            self.links.append(link)
            return link
        }, notifier: notifier, ledger: ledger, held: held, words: words, deadline: deadline)
    }

    func testTargetNeedsTheRoomTheAgentAndTheTurn() {
        XCTAssertNil(ReplyTarget(userInfo: ["room": room, "device_id": "d", "pane_id": "p"], notificationId: "n"))
        XCTAssertNil(ReplyTarget(userInfo: ["room": "short", "device_id": "d", "pane_id": "p", "turn": "1"], notificationId: "n"))
        XCTAssertNil(ReplyTarget(userInfo: ["room": room, "device_id": "d", "pane_id": "p", "turn": "12a"], notificationId: "n"))
        XCTAssertNotNil(ReplyTarget(userInfo: ["room": room, "device_id": "d", "pane_id": "p", "turn": "12"], notificationId: "n"))
    }

    func testRequestIdIsStablePerNotificationAndWithinWhatTheComputerAccepts() {
        XCTAssertEqual(target("n1").requestId, target("n1").requestId)
        XCTAssertNotEqual(target("n1").requestId, target("n2").requestId)
        let id = target().requestId
        XCTAssertLessThanOrEqual(id.count, 64)
        XCTAssertTrue(id.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-" || $0 == "_") })
    }

    func testADeliveredReplyCarriesTheTurnAndEndsQuietly() async {
        let s = sender([.answer(["type": "input_result", "ok": true])])
        await s.reply(text: "yes", target: target())
        XCTAssertEqual(links.count, 1)
        let message = links[0].sent[0]
        XCTAssertEqual(message["type"] as? String, "input")
        XCTAssertEqual(message["text"] as? String, "yes")
        XCTAssertEqual(message["turn"] as? String, "1700000000123")
        XCTAssertEqual(message["request_id"] as? String, target().requestId)
        XCTAssertEqual(links[0].order, ["connect", "request"])
        XCTAssertTrue(links[0].closed)
        XCTAssertEqual(ledger.states[target().requestId], .sent)
        XCTAssertTrue(notifier.posted.isEmpty)
        XCTAssertTrue(notifier.scheduled.isEmpty)
        XCTAssertNil(held.take(room: room, deviceId: "dev", paneId: "pane"))
    }

    func testTheDeadManNoticeIsScheduledBeforeTheSendAndTheLedgerIsWrittenBeforeBoth() async {
        let s = sender([.answer(["ok": true])])
        await s.reply(text: "yes", target: target())
        XCTAssertEqual(ledger.log.first, "sending")
        XCTAssertEqual(notifier.events.first, "schedule hide.reply.unconfirmed.\(target().requestId)")
        XCTAssertEqual(notifier.events.last, "cancel hide.reply.unconfirmed.\(target().requestId)")
    }

    func testTheDeadManNoticeIsSetForFortySecondsInTheWordsOfAnUnconfirmedReply() async {
        let s = sender([.answer(["ok": true])])
        await s.reply(text: "yes", target: target())
        let notice = notifier.everScheduled["hide.reply.unconfirmed.\(target().requestId)"]
        XCTAssertEqual(notice?.after, 40)
        XCTAssertEqual(notice?.body, "unconfirmed")
    }

    func testAComputerThatCannotBeReachedMeansNotSentAndKeepsTheText() async {
        let s = sender([.connectFails])
        await s.reply(text: "my words", target: target())
        XCTAssertTrue(links[0].sent.isEmpty)
        XCTAssertEqual(notifier.posted.values.first, "not-sent")
        XCTAssertEqual(held.take(room: room, deviceId: "dev", paneId: "pane"), "my words")
        XCTAssertEqual(ledger.states[target().requestId], .failed)
    }

    func testALockedPhoneOrAMissingPairingIsNotSent() async {
        await sender([], pairing: .failure(Locked())).reply(text: "a", target: target("n1"))
        await sender([], pairing: .success(nil)).reply(text: "b", target: target("n2"))
        XCTAssertTrue(links.isEmpty)
        XCTAssertEqual(notifier.posted.count, 2)
        XCTAssertEqual(Set(notifier.posted.values), ["not-sent"])
    }

    func testAnAskThatMovedOnIsToldAsNoLongerWaiting() async {
        await sender([.answer(["ok": false, "reason": "stale"])]).reply(text: "yes", target: target())
        XCTAssertEqual(notifier.posted.values.first, "stale")
        XCTAssertEqual(held.take(room: room, deviceId: "dev", paneId: "pane"), "yes")
    }

    func testAnUnansweredSendIsUncertainAndIsNeverSentAgain() async {
        let s = sender([.requestFails(.unconfirmed), .answer(["ok": true])])
        await s.reply(text: "yes", target: target())
        XCTAssertEqual(notifier.posted.values.first, "unconfirmed")
        XCTAssertEqual(ledger.states[target().requestId], .uncertain)
        // The same action delivered again does not send a second time.
        await s.reply(text: "yes", target: target())
        XCTAssertEqual(links.count, 1)
    }

    func testTheComputersOwnUncertainAnswerIsTheSameNotice() async {
        await sender([.answer(["ok": false, "reason": "uncertain"])]).reply(text: "yes", target: target())
        XCTAssertEqual(notifier.posted.values.first, "unconfirmed")
        XCTAssertEqual(ledger.states[target().requestId], .uncertain)
    }

    func testAfterTheHandoverOnlyTheComputersRefusalIsCertain() {
        // A peer that left after the send, an answer that could not be read and a closed socket all surface from the
        // link as `.notSent` or as a plain error; once the message was handed over, none of them says "not sent".
        XCTAssertEqual(ReplyRule.afterHandover(LinkFailure.notSent), .unconfirmed)
        XCTAssertEqual(ReplyRule.afterHandover(LinkFailure.unconfirmed), .unconfirmed)
        XCTAssertEqual(ReplyRule.afterHandover(URLError(.networkConnectionLost)), .unconfirmed)
        XCTAssertEqual(ReplyRule.afterHandover(LinkFailure.answer("stale")), .unconfirmed)
        XCTAssertEqual(ReplyRule.afterHandover(LinkFailure.outdated), .outdated)
    }

    func testOnlyAnEndThatCertainlyWroteNothingReturnsTheText() {
        for failure: LinkFailure in [.notSent, .outdated, .answer("stale"), .answer("gone"), .answer("failed")] {
            XCTAssertTrue(ReplyRule.returnsText(failure), "\(failure)")
        }
        for failure: LinkFailure in [.unconfirmed, .answer("uncertain"), .answer("in_flight")] {
            XCTAssertFalse(ReplyRule.returnsText(failure), "\(failure)")
        }
    }

    func testAReplyThatMostLikelyArrivedIsNeverPutBackInTheBox() async {
        await sender([.requestFails(.unconfirmed)]).reply(text: "yes", target: target())
        XCTAssertNil(held.take(room: room, deviceId: "dev", paneId: "pane"))
        let again = sender([.answer(["ok": false, "reason": "in_flight"])])
        await again.reply(text: "yes", target: target("n2"))
        XCTAssertNil(held.take(room: room, deviceId: "dev", paneId: "pane"))
    }

    func testAReplyThatCertainlyWroteNothingIsPutBack() async {
        await sender([.answer(["ok": false, "reason": "stale"])]).reply(text: "yes", target: target())
        XCTAssertEqual(held.take(room: room, deviceId: "dev", paneId: "pane"), "yes")
    }

    func testAnOlderComputerIsToldToUpdate() async {
        await sender([.requestFails(.outdated)]).reply(text: "yes", target: target())
        XCTAssertEqual(notifier.posted.values.first, "outdated")
    }

    func testADeliveredActionDeliveredAgainIsNotSentAgain() async {
        let s = sender([.answer(["ok": true]), .answer(["ok": true])])
        await s.reply(text: "yes", target: target())
        await s.reply(text: "yes", target: target())
        XCTAssertEqual(links.count, 1)
    }

    func testACrashedSendIsRetriedWithTheSameRequestId() async {
        ledger.states[target().requestId] = .sending
        let s = sender([.answer(["ok": true])])
        await s.reply(text: "yes", target: target())
        XCTAssertEqual(links[0].sent[0]["request_id"] as? String, target().requestId)
        XCTAssertEqual(ledger.states[target().requestId], .sent)
    }

    func testTheDeadlineEndsAHangingSendAsUncertain() async {
        let s = sender([.hang], deadline: 0.05)
        await s.reply(text: "yes", target: target())
        XCTAssertTrue(links[0].closed)
        XCTAssertEqual(notifier.posted.values.first, "unconfirmed")
    }

    func testAnEmptyReplyIsNotSent() async {
        let s = sender([.answer(["ok": true])])
        await s.reply(text: "  \n", target: target())
        XCTAssertTrue(links.isEmpty)
        XCTAssertTrue(notifier.events.isEmpty)
    }
}
