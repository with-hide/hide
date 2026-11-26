import CryptoKit
import XCTest
@testable import HidePushCore

final class NoticeTests: XCTestCase {
    let room = String(repeating: "A", count: 43)
    let otherRoom = String(repeating: "B", count: 43)

    func key(_ room: String) -> PushKey {
        PushKey(room: room, privateKey: P256.KeyAgreement.PrivateKey().rawRepresentation, auth: Data((0..<16).map { UInt8($0) }))
    }

    func notice(room: String, extra: [String: Any] = [:]) -> Data {
        var object: [String: Any] = [
            "title": "fix the build", "state": "needs_you", "status_code": "question", "detail": "Which branch?", "place": "contong",
            "tag": "mini|w2:p1", "device_id": "mini", "pane_id": "w2:p1", "room": room, "clear": [],
            "ask": ["kind": "question"],
        ]
        object.merge(extra) { $1 }
        return try! JSONSerialization.data(withJSONObject: object)
    }

    func testOpensTheNoticeWithTheKeyOfItsComputerAmongSeveral() throws {
        let mine = key(room), theirs = key(otherRoom)
        let body = try encrypt(notice(room: room), for: mine)
        let opened = try XCTUnwrap(NoticeOpener.open(hide: body, keys: [theirs, mine]))
        XCTAssertEqual(opened.title, "fix the build")
        XCTAssertEqual(NoticeInfo.of(opened), ["room": room, "device_id": "mini", "pane_id": "w2:p1"])
    }

    func testStaysClosedWithNoKeyWithAKeyOfAnotherComputerAndForARoomTheKeyIsNot() throws {
        let mine = key(room)
        let body = try encrypt(notice(room: room), for: mine)
        XCTAssertNil(NoticeOpener.open(hide: body, keys: []), "a computer that was removed has no key")
        XCTAssertNil(NoticeOpener.open(hide: body, keys: [key(otherRoom)]))
        XCTAssertNil(NoticeOpener.open(hide: "not base64 !", keys: [mine]))
        let wrongRoom = try encrypt(notice(room: otherRoom), for: mine)
        XCTAssertNil(NoticeOpener.open(hide: wrongRoom, keys: [mine]), "this key's computer did not send a notice naming another")
    }

    func testReadsNoOddlyShapedNoticeAndIgnoresTheOlderComputersAsk() throws {
        XCTAssertNil(Notice(json: notice(room: room, extra: ["title": ""])))
        XCTAssertNil(Notice(json: notice(room: room, extra: ["room": "short"])))
        XCTAssertNil(Notice(json: Data(repeating: 0x20, count: 5000)))
        let asks: [Any] = [["kind": "question", "turn": "12ab"], ["kind": "approval", "turn": "5"], NSNull()]
        for ask in asks {
            let opened = try XCTUnwrap(Notice(json: notice(room: room, extra: ["ask": ask])))
            XCTAssertEqual(NoticeInfo.of(opened), ["room": room, "device_id": "mini", "pane_id": "w2:p1"])
        }
    }

    func testWordsTheBodyInThePhonesLanguageAndFallsBackToEnglish() throws {
        let asking = try XCTUnwrap(Notice(json: notice(room: room)))
        XCTAssertEqual(asking.body(words: NoticeWords.forLanguages(["ko-KR"])), "확인할 것 · Which branch? · contong")
        XCTAssertEqual(asking.body(words: NoticeWords.forLanguages(["en-US"])), "To check · Which branch? · contong")
        XCTAssertEqual(asking.body(words: NoticeWords.forLanguages(["fr-FR", "ko"])), "확인할 것 · Which branch? · contong")
        XCTAssertEqual(asking.body(words: NoticeWords.forLanguages(["fr-FR"])), "To check · Which branch? · contong")
        let letter = try XCTUnwrap(Notice(json: notice(room: room, extra: ["state": "letter_undelivered", "status_code": NSNull(), "detail": NSNull(), "place": "letter-7"])))
        XCTAssertEqual(letter.body(words: NoticeWords.forLanguages(["en"])), "Letter not delivered · letter-7")
    }

    func testReadsANoticeAboutTheComputerItselfWhichNamesNoAgent() throws {
        let paired = try XCTUnwrap(Notice(json: notice(room: room, extra: [
            "title": "build-box", "state": "phone_paired", "status_code": NSNull(), "detail": "Pixel 9", "place": "",
            "tag": "|", "device_id": "", "pane_id": "",
        ])))
        XCTAssertEqual(paired.body(words: NoticeWords.forLanguages(["ko"])), "새 폰이 연결됐어요 · Pixel 9")
        XCTAssertEqual(NoticeInfo.of(paired), ["room": room, "device_id": "", "pane_id": ""])
        XCTAssertNil(Notice(json: notice(room: room, extra: ["device_id": ""])), "an agent's notice names both its device and its pane")
        XCTAssertNil(Notice(json: notice(room: room, extra: ["pane_id": ""])))
    }

    func testTheDemoSampleIsWordedAsARealQuestionIs() {
        let sample = Notice(sampleTitle: "Address review comments", statusCode: "question", state: "needs_you", detail: "Apply all 3 review comments?", place: "demo · fix/review")
        XCTAssertEqual(sample.title, "Address review comments")
        XCTAssertEqual(sample.body(words: NoticeWords.forLanguages(["en"])), "To check · Apply all 3 review comments? · demo · fix/review")
    }
}
