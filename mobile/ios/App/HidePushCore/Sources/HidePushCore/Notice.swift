import Foundation

/// What hided sends the phone about one agent, as `push::payload` builds it (`hided/src/mobile/push.rs`), read for the
/// parts a notification uses. The rest of the JSON (the tags to close, the subject) is not kept.
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
    /// The largest notice the app reads: hided's own plaintext bound is 2,897 bytes (`push::MAX_APP_PLAINTEXT`).
    static let maxBytes = 4096

    /// The notice of the demo's sample question: the shape a real one has, for a computer that does not exist, so the
    /// title, the body and the category come from the same code (`body(words:)`, `NoticeCategory`) as a real notification's.
    public init(sampleTitle title: String, statusCode: String, state: String, detail: String, place: String) {
        self.title = title
        self.state = state
        self.statusCode = statusCode
        self.detail = detail
        self.place = place
        self.tag = "demo"
        self.deviceId = "demo"
        self.paneId = "demo"
        self.room = ""
    }

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
    }
}

/// The notification category the app registers and the extension names.
public enum NoticeCategory {
    /// A tap opens the conversation; all notices have no actions.
    public static let open = "hide.open"
}

/// The keys of the `userInfo` a decrypted notification carries: what a tap needs to find the agent,
/// and nothing the computer sent besides.
public enum NoticeInfo {
    public static let room = "room"
    public static let deviceId = "device_id"
    public static let paneId = "pane_id"

    public static func of(_ notice: Notice) -> [String: String] {
        [room: notice.room, deviceId: notice.deviceId, paneId: notice.paneId]
    }
}

public extension Notice {
    /// The body: the lead word, then the computer's sentence, then the project, as the web app's worker joins them.
    func body(words: NoticeWords) -> String {
        [words.lead(statusCode: statusCode, state: state), detail, place.isEmpty ? nil : place]
            .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
    }
}
