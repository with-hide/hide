import Foundation

/// What hided sends the phone about one agent, as `push::payload` builds it (`hided/src/mobile/push.rs`), read for the
/// parts a notification and its actions use. The rest of the JSON (the tags to close, the subject) is not kept.
public struct Notice: Equatable {
    public let title: String
    public let state: String
    public let statusCode: String?
    public let detail: String?
    public let place: String
    public let tag: String
    public let deviceId: String
    public let paneId: String
    public let room: String
    /// `question` or `approval`, and the agent's last change of state the notice was made at; the action sent later
    /// carries `askTurn` and hided refuses it once the agent is no longer in that ask (`stale`).
    public let askKind: String?
    public let askTurn: String?

    /// The largest notice the app reads: hided's own plaintext bound is 2,897 bytes (`push::MAX_APNS_PLAINTEXT`).
    static let maxBytes = 4096

    public init?(json: Data) {
        guard json.count <= Self.maxBytes,
              let object = (try? JSONSerialization.jsonObject(with: json)) as? [String: Any],
              let title = object["title"] as? String, !title.isEmpty, title.count <= 512,
              let state = object["state"] as? String, state.count <= 64,
              let tag = object["tag"] as? String, !tag.isEmpty, tag.count <= 768,
              let deviceId = object["device_id"] as? String, !deviceId.isEmpty, deviceId.count <= 256,
              let paneId = object["pane_id"] as? String, !paneId.isEmpty, paneId.count <= 512,
              let room = object["room"] as? String, PushKeychain.isRoom(room) else { return nil }
        self.title = title
        self.state = state
        self.statusCode = (object["status_code"] as? String).flatMap { $0.count <= 64 ? $0 : nil }
        self.detail = (object["detail"] as? String).flatMap { $0.isEmpty || $0.count > 1024 ? nil : $0 }
        self.place = (object["place"] as? String).flatMap { $0.count <= 512 ? $0 : nil } ?? ""
        self.tag = tag
        self.deviceId = deviceId
        self.paneId = paneId
        self.room = room
        if let ask = object["ask"] as? [String: Any], let kind = ask["kind"] as? String, ["question", "approval"].contains(kind),
           let turn = ask["turn"] as? String, !turn.isEmpty, turn.count <= 32, turn.allSatisfy(\.isNumber) {
            askKind = kind
            askTurn = turn
        } else {
            askKind = nil
            askTurn = nil
        }
    }
}

/// The notification categories the app registers and the extension names; an action exists only for a category.
public enum NoticeCategory {
    /// The agent waits on text: Reply.
    public static let reply = "hide.reply"
    /// Anything else: a tap opens the agent and there is no action.
    public static let open = "hide.open"

    public static func of(_ notice: Notice) -> String {
        notice.askKind == "question" ? reply : open
    }
}

/// The keys of the `userInfo` a decrypted notification carries: what a tap and an action need to find the agent,
/// and nothing the computer sent besides.
public enum NoticeInfo {
    public static let room = "room"
    public static let deviceId = "device_id"
    public static let paneId = "pane_id"
    public static let turn = "turn"

    public static func of(_ notice: Notice) -> [String: String] {
        var info = [room: notice.room, deviceId: notice.deviceId, paneId: notice.paneId]
        if let turn = notice.askTurn { info[Self.turn] = turn }
        return info
    }
}

public extension Notice {
    /// The body: the lead word, then the computer's sentence, then the project, as the web app's worker joins them.
    func body(words: NoticeWords) -> String {
        [words.lead(statusCode: statusCode, state: state), detail, place.isEmpty ? nil : place]
            .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
    }
}
