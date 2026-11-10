import CryptoKit
import Foundation

/// The agent a Reply on a notification is for: the room, the agent and the ask the notice was made at (`ask.turn`),
/// all taken from the `userInfo` the extension wrote after it decrypted the notice. A notification the extension could
/// not open carries none of it, and so has no Reply.
public struct ReplyTarget: Equatable {
    public let room: String
    public let deviceId: String
    public let paneId: String
    public let turn: String
    public let notificationId: String

    public init?(userInfo: [AnyHashable: Any], notificationId: String) {
        guard let room = userInfo[NoticeInfo.room] as? String, PushKeychain.isRoom(room),
              let device = userInfo[NoticeInfo.deviceId] as? String, !device.isEmpty, device.count <= 256,
              let pane = userInfo[NoticeInfo.paneId] as? String, !pane.isEmpty, pane.count <= 512,
              let turn = userInfo[NoticeInfo.turn] as? String, !turn.isEmpty, turn.count <= 32, turn.allSatisfy(\.isNumber),
              !notificationId.isEmpty else { return nil }
        self.room = room
        self.deviceId = device
        self.paneId = pane
        self.turn = turn
        self.notificationId = notificationId
    }

    /// One id per notification and action, so a repeat of the same action is the same request: hided's claim on the
    /// id answers a repeat without writing the reply again. 40 hex digits fit the 64 characters hided allows.
    public var requestId: String {
        let digest = SHA256.hash(data: Data(("reply|" + notificationId).utf8))
        return "n-" + digest.map { String(format: "%02x", $0) }.joined().prefix(40)
    }

    /// What a failure notice carries so a tap on it opens the same agent.
    public var userInfo: [String: String] {
        [NoticeInfo.room: room, NoticeInfo.deviceId: deviceId, NoticeInfo.paneId: paneId]
    }
}

/// Why a reply did not reach the computer, in the words a person can act on.
public enum LinkFailure: Error, Equatable {
    /// Nothing reached the computer: offline, the computer refused the phone, the handshake failed.
    case notSent
    /// The computer is older than this app's notifications.
    case outdated
    /// The reply was handed to the connection and no answer came back: it may have arrived.
    case unconfirmed
    /// The computer answered `input_result` with `ok: false` and this reason.
    case answer(String)
}

/// The one rule for what became of a reply, so no path decides it on its own. A reply has three ends only:
/// not sent (the failure is certain to have come before anything was handed to the connection), sent (the computer
/// answered `input_result` with `ok: true`), and not confirmed (everything else: a peer that left after the send, an
/// answer that could not be read, the deadline, a closed socket). Only the first, and the computer's own refusals that
/// say nothing was written, give the text back to the person; a reply that most likely arrived is never offered again.
public enum ReplyRule {
    /// What any end of the wait means once the message was handed to the connection: only the computer's refusal of
    /// the message as unknown (`outdated`) is certain; every other way the wait ended is `unconfirmed`.
    public static func afterHandover(_ error: Error) -> LinkFailure {
        if let failure = error as? LinkFailure, failure == .outdated { return .outdated }
        return .unconfirmed
    }

    /// The computer could not say whether it wrote the reply.
    public static func isUncertain(_ failure: LinkFailure) -> Bool {
        switch failure {
        case .unconfirmed, .answer("uncertain"), .answer("in_flight"): return true
        default: return false
        }
    }

    /// The text goes back into the person's reply box only for an end that certainly wrote nothing.
    public static func returnsText(_ failure: LinkFailure) -> Bool { !isUncertain(failure) }
}

/// One encrypted session to one computer, for as long as one action takes. The real one is `RelayPhoneLink` (over
/// the relay protocol library), a test's is a stand-in; both end every wait when `close` is called.
public protocol PhoneLink: AnyObject {
    /// Joins the room, finishes the handshake, signs in with the credential and waits for the computer's hello.
    /// Throws `LinkFailure.notSent` for anything that goes wrong on the way: nothing has been sent yet.
    func connect(room: String, noiseKey: Data, credential: String) async throws
    /// Sends one message and waits for the message of `type` that names `requestId`. Throws `.notSent` only when the
    /// message could not be handed to the connection, and after that only `.outdated` (the computer refused the message
    /// as unknown) or `.unconfirmed` (`ReplyRule.afterHandover`): whatever ended the wait, the message may have arrived.
    func request(_ message: [String: Any], resultType: String, requestId: String) async throws -> [String: Any]
    func close()
}

public protocol PairingSource {
    /// The computer's connection secrets, or nil when the phone has none for this room; throws when the Keychain cannot
    /// be read (a locked phone).
    func pairing(room: String) throws -> (noiseKey: Data, credential: String)?
}

public protocol ReplyNotifier: AnyObject {
    func schedule(id: String, body: String, userInfo: [String: String], after: TimeInterval)
    func cancel(id: String)
    func post(id: String, body: String, userInfo: [String: String])
}

public enum ReplyState: String, Equatable {
    /// Written before the send: a crash after this leaves a mark that the reply may be on its way.
    case sending
    case sent
    /// The computer could not tell: never sent again by the app on its own.
    case uncertain
    case failed
}

public protocol ReplyLedger: AnyObject {
    func state(of requestId: String) -> ReplyState?
    func set(_ state: ReplyState, for requestId: String)
}

/// The text of a reply that did not arrive, kept for an hour so the person does not type it again.
public protocol HeldReplies: AnyObject {
    func keep(text: String, room: String, deviceId: String, paneId: String)
    func take(room: String, deviceId: String, paneId: String) -> String?
    func drop(room: String, deviceId: String, paneId: String)
}

public struct ReplyWords: Equatable {
    public let stale: String
    public let notSent: String
    public let outdated: String
    public let unconfirmed: String

    public init(stale: String, notSent: String, outdated: String, unconfirmed: String) {
        self.stale = stale
        self.notSent = notSent
        self.outdated = outdated
        self.unconfirmed = unconfirmed
    }
}

/// Sends the reply of a notification to the computer without the app being opened, and tells the person when it did
/// not arrive. It never sends twice (the request id is stable and the ledger is written before the send), never fails
/// silently (a local notification is scheduled before the send and replaced by the exact answer) and never logs the
/// text.
public final class ReplySender {
    public static let deadManAfter: TimeInterval = 40
    public static let deadline: TimeInterval = 25

    private let pairing: PairingSource
    private let makeLink: () -> PhoneLink
    private let notifier: ReplyNotifier
    private let ledger: ReplyLedger
    private let held: HeldReplies
    private let words: ReplyWords
    private let deadManAfter: TimeInterval
    private let deadline: TimeInterval

    public init(pairing: PairingSource, makeLink: @escaping () -> PhoneLink, notifier: ReplyNotifier, ledger: ReplyLedger,
                held: HeldReplies, words: ReplyWords, deadManAfter: TimeInterval = ReplySender.deadManAfter,
                deadline: TimeInterval = ReplySender.deadline) {
        self.pairing = pairing
        self.makeLink = makeLink
        self.notifier = notifier
        self.ledger = ledger
        self.held = held
        self.words = words
        self.deadManAfter = deadManAfter
        self.deadline = deadline
    }

    static func unconfirmedId(_ requestId: String) -> String { "hide.reply.unconfirmed." + requestId }
    static func failedId(_ requestId: String) -> String { "hide.reply.failed." + requestId }

    public func reply(text: String, target: ReplyTarget) async {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        let id = target.requestId
        switch ledger.state(of: id) {
        case .sent, .uncertain: return
        case .sending, .failed, nil: break
        }
        ledger.set(.sending, for: id)
        // Before the send, so a process that is killed in the middle of it still ends in a notice.
        notifier.schedule(id: Self.unconfirmedId(id), body: words.unconfirmed, userInfo: target.userInfo, after: deadManAfter)
        let failure = await attempt(text: text, target: target)
        notifier.cancel(id: Self.unconfirmedId(id))
        guard let failure else {
            ledger.set(.sent, for: id)
            held.drop(room: target.room, deviceId: target.deviceId, paneId: target.paneId)
            return
        }
        if ReplyRule.returnsText(failure) {
            held.keep(text: text, room: target.room, deviceId: target.deviceId, paneId: target.paneId)
        }
        if ReplyRule.isUncertain(failure) {
            ledger.set(.uncertain, for: id)
            notifier.post(id: Self.failedId(id), body: words.unconfirmed, userInfo: target.userInfo)
            return
        }
        switch failure {
        case .answer("stale"), .answer("gone"):
            ledger.set(.failed, for: id)
            notifier.post(id: Self.failedId(id), body: words.stale, userInfo: target.userInfo)
        case .outdated:
            ledger.set(.failed, for: id)
            notifier.post(id: Self.failedId(id), body: words.outdated, userInfo: target.userInfo)
        case .notSent, .answer, .unconfirmed:
            ledger.set(.failed, for: id)
            notifier.post(id: Self.failedId(id), body: words.notSent, userInfo: target.userInfo)
        }
    }

    /// Nil when the computer wrote the reply; otherwise why not.
    private func attempt(text: String, target: ReplyTarget) async -> LinkFailure? {
        let secrets: (noiseKey: Data, credential: String)
        do {
            guard let found = try pairing.pairing(room: target.room) else { return .notSent }
            secrets = found
        } catch { return .notSent }
        let link = makeLink()
        // The deadline closes the link, which ends whichever wait is open; the link names what that means.
        let timer = Task { [deadline] in
            try? await Task.sleep(nanoseconds: UInt64(deadline * 1_000_000_000))
            if !Task.isCancelled { link.close() }
        }
        defer { timer.cancel(); link.close() }
        do {
            try await link.connect(room: target.room, noiseKey: secrets.noiseKey, credential: secrets.credential)
            let answer = try await link.request(
                ["type": "input", "request_id": target.requestId, "device_id": target.deviceId, "pane_id": target.paneId,
                 "text": text, "turn": target.turn],
                resultType: "input_result", requestId: target.requestId)
            if answer["ok"] as? Bool == true { return nil }
            return .answer(answer["reason"] as? String ?? "failed")
        } catch let failure as LinkFailure {
            return failure
        } catch { return .notSent }
    }
}
