import CryptoKit
import Foundation
import Security
import UIKit
import UserNotifications
import HidePushCore

/// The computer's connection secrets for an action: read from the app's own Keychain group, which holds the one thing a
/// locked phone must not give (`WhenUnlockedThisDeviceOnly`). An action asks for Face ID or the passcode first, so the
/// read succeeds; on a locked phone it throws and the reply is told as not sent.
public struct KeychainPairingSource: PairingSource {
    private let keychain: PairingKeychain

    public init(keychain: PairingKeychain = PairingKeychain(accessGroup: PairingKeychain.ownAccessGroup())) {
        self.keychain = keychain
    }

    public func pairing(room: String) throws -> (noiseKey: Data, credential: String)? {
        guard let item = try keychain.read(room: room), let credential = item.credential, let key = Base64URL.decode(item.noiseKey),
              key.count == 32 else { return nil }
        return (key, credential)
    }
}

/// What the app knows of each action it ran, in the app's own defaults (a delete of the app takes it away): the mark
/// written before a send, and how it ended. Bounded and short lived: 64 entries, a day.
public final class DefaultsReplyLedger: ReplyLedger {
    private let defaults: UserDefaults
    private let key: String
    private let now: () -> Date
    private let lock = NSLock()
    static let limit = 64
    static let maxAge: TimeInterval = 24 * 3600

    public init(defaults: UserDefaults = .standard, key: String = "me.grab.hide.ios.reply-ledger.v1", now: @escaping () -> Date = Date.init) {
        self.defaults = defaults
        self.key = key
        self.now = now
    }

    private func read() -> [String: [String: Any]] {
        (defaults.dictionary(forKey: key) as? [String: [String: Any]]) ?? [:]
    }

    public func state(of requestId: String) -> ReplyState? {
        lock.lock(); defer { lock.unlock() }
        guard let entry = read()[requestId], let raw = entry["state"] as? String, let at = entry["at"] as? Double,
              now().timeIntervalSince1970 - at < Self.maxAge else { return nil }
        return ReplyState(rawValue: raw)
    }

    public func set(_ state: ReplyState, for requestId: String) {
        lock.lock(); defer { lock.unlock() }
        var all = read()
        let at = now().timeIntervalSince1970
        all[requestId] = ["state": state.rawValue, "at": at]
        all = all.filter { at - (($0.value["at"] as? Double) ?? 0) < Self.maxAge }
        if all.count > Self.limit {
            for stale in all.sorted(by: { (($0.value["at"] as? Double) ?? 0) < (($1.value["at"] as? Double) ?? 0) }).prefix(all.count - Self.limit) {
                all[stale.key] = nil
            }
        }
        defaults.set(all, forKey: key)
    }
}

/// The text of a reply that did not arrive, for an hour, in the app's own Keychain group: it is something the person
/// wrote, and a copy in a plain file or the defaults would outlive the hour and be readable from a backup. One item per
/// agent, named by a hash of its address, so nothing in the Keychain names a computer or an agent. At most eight.
public final class KeychainHeldReplies: HeldReplies {
    private let service: String
    private let accessGroup: String?
    private let now: () -> Date
    private let lock = NSLock()
    static let maxItems = 8
    static let lifetime: TimeInterval = 3600

    public init(service: String = "me.grab.hide.ios.held-reply", accessGroup: String? = PairingKeychain.ownAccessGroup(), now: @escaping () -> Date = Date.init) {
        self.service = service
        self.accessGroup = accessGroup
        self.now = now
    }

    private func account(_ room: String, _ deviceId: String, _ paneId: String) -> String {
        SHA256.hash(data: Data("\(room)|\(deviceId)|\(paneId)".utf8)).map { String(format: "%02x", $0) }.joined()
    }

    private func base(_ account: String?) -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrSynchronizable as String: kCFBooleanFalse as Any,
        ]
        if let accessGroup { query[kSecAttrAccessGroup as String] = accessGroup }
        if let account { query[kSecAttrAccount as String] = account }
        return query
    }

    public func keep(text: String, room: String, deviceId: String, paneId: String) {
        lock.lock(); defer { lock.unlock() }
        pruneLocked()
        let name = account(room, deviceId, paneId)
        guard let data = try? JSONSerialization.data(withJSONObject: ["text": text, "until": now().timeIntervalSince1970 + Self.lifetime]) else { return }
        SecItemDelete(base(name) as CFDictionary)
        if allLocked().count >= Self.maxItems, let oldest = allLocked().min(by: { $0.until < $1.until }) {
            SecItemDelete(base(oldest.account) as CFDictionary)
        }
        var add = base(name)
        add[kSecValueData as String] = data
        // After the first unlock: the reply fails some seconds after the Face ID that allowed it, and the phone may have locked again.
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(add as CFDictionary, nil)
    }

    public func take(room: String, deviceId: String, paneId: String) -> String? {
        lock.lock(); defer { lock.unlock() }
        pruneLocked()
        var query = base(account(room, deviceId, paneId))
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess, let data = result as? Data,
              let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any], let text = body["text"] as? String else { return nil }
        return text
    }

    public func drop(room: String, deviceId: String, paneId: String) {
        lock.lock(); defer { lock.unlock() }
        SecItemDelete(base(account(room, deviceId, paneId)) as CFDictionary)
    }

    private struct Held { let account: String; let until: Double }

    private func allLocked() -> [Held] {
        var query = base(nil)
        query[kSecReturnAttributes as String] = true
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitAll
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess, let rows = result as? [[String: Any]] else { return [] }
        return rows.compactMap { row in
            guard let account = row[kSecAttrAccount as String] as? String, let data = row[kSecValueData as String] as? Data,
                  let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any], let until = body["until"] as? Double else {
                return (row[kSecAttrAccount as String] as? String).map { Held(account: $0, until: 0) }
            }
            return Held(account: account, until: until)
        }
    }

    private func pruneLocked() {
        let at = now().timeIntervalSince1970
        for held in allLocked() where held.until <= at { SecItemDelete(base(held.account) as CFDictionary) }
    }
}

/// Local notifications for the outcome of a reply, through the app's notification center. Each carries the agent's
/// address in the way the extension writes it, so a tap opens that agent, and no category, so it has no action.
public final class CenterReplyNotifier: ReplyNotifier {
    private let center: UNUserNotificationCenter

    public init(center: UNUserNotificationCenter = .current()) { self.center = center }

    private func request(id: String, body: String, userInfo: [String: String], trigger: UNNotificationTrigger?) -> UNNotificationRequest {
        let content = UNMutableNotificationContent()
        content.title = NSLocalizedString("push.notice.title", comment: "Title of a notice the app posts about a reply")
        content.body = body
        content.categoryIdentifier = NoticeCategory.open
        content.userInfo = userInfo
        return UNNotificationRequest(identifier: id, content: content, trigger: trigger)
    }

    public func schedule(id: String, body: String, userInfo: [String: String], after: TimeInterval) {
        center.add(request(id: id, body: body, userInfo: userInfo, trigger: UNTimeIntervalNotificationTrigger(timeInterval: after, repeats: false)))
    }

    public func cancel(id: String) {
        center.removePendingNotificationRequests(withIdentifiers: [id])
        center.removeDeliveredNotifications(withIdentifiers: [id])
    }

    public func post(id: String, body: String, userInfo: [String: String]) {
        center.add(request(id: id, body: body, userInfo: userInfo, trigger: nil))
    }
}

/// A Reply on a notification, answered in the app's process whether or not a window exists: the work runs under a
/// background task, and the notification center's completion handler is called only after the outcome is told.
public final class PushActions: PushActionHandling {
    private let sender: ReplySender
    private let demo: (UNNotification, String) -> Void

    public init(sender: ReplySender, demo: @escaping (UNNotification, String) -> Void = { _, _ in }) {
        self.sender = sender
        self.demo = demo
    }

    public static func live() -> PushActions {
        let notifier = CenterReplyNotifier()
        let url = RelayPhoneLink.relayURL() ?? URL(string: RelayPhoneLink.defaultURL)!
        let sender = ReplySender(pairing: KeychainPairingSource(), makeLink: { RelayPhoneLink(url: url) }, notifier: notifier,
                                 ledger: DefaultsReplyLedger(), held: KeychainHeldReplies(), words: NoticeWords.current.reply)
        return PushActions(sender: sender, demo: { notification, text in
            // The demo notification's reply acts locally: it names no computer and opens no connection.
            notifier.post(id: "hide.demo.sent." + notification.request.identifier, body: NSLocalizedString("push.demo.sent", comment: "Said after a reply to the sample notification"), userInfo: [:])
        })
    }

    public func reply(text: String, notification: UNNotification, completion: @escaping () -> Void) {
        let userInfo = notification.request.content.userInfo
        if userInfo["demo"] as? Bool == true {
            demo(notification, text)
            completion()
            return
        }
        guard let target = ReplyTarget(userInfo: userInfo, notificationId: notification.request.identifier) else {
            completion()
            return
        }
        var background = UIBackgroundTaskIdentifier.invalid
        let finish = {
            if background != .invalid { UIApplication.shared.endBackgroundTask(background); background = .invalid }
            completion()
        }
        background = UIApplication.shared.beginBackgroundTask(withName: "hide.reply") { finish() }
        let sender = self.sender
        Task {
            await sender.reply(text: text, target: target)
            await MainActor.run { finish() }
        }
    }
}
