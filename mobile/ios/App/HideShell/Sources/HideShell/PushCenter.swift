import Foundation
import UIKit
import UserNotifications
import HidePushCore

/// The agent a tapped notification names, held until the page asks for it, once. The notice a decrypted
/// notification carried (`NoticeInfo`) is all it holds, and a notification the extension could not open carries none,
/// so a tap on it opens the app and no agent.
public final class PushOpenInbox {
    public static let shared = PushOpenInbox()

    public struct Target: Equatable {
        public let room: String
        public let deviceId: String
        public let paneId: String
    }

    private var target: Target?
    private let lock = NSLock()

    public init() {}

    /// Keeps the target of `userInfo` when it names an agent of a computer by the shapes the extension writes.
    @discardableResult
    public func offer(_ userInfo: [AnyHashable: Any]) -> Bool {
        guard let room = userInfo[NoticeInfo.room] as? String, PushKeychain.isRoom(room),
              let device = userInfo[NoticeInfo.deviceId] as? String, !device.isEmpty, device.count <= 256,
              let pane = userInfo[NoticeInfo.paneId] as? String, !pane.isEmpty, pane.count <= 512 else { return false }
        lock.lock(); defer { lock.unlock() }
        target = Target(room: room, deviceId: device, paneId: pane)
        NotificationCenter.default.post(name: .hidePushOpenArrived, object: self)
        return true
    }

    public func take() -> Target? {
        lock.lock(); defer { lock.unlock() }
        defer { target = nil }
        return target
    }
}

public extension Notification.Name {
    static let hidePushOpenArrived = Notification.Name("me.grab.hide.ios.push-open-arrived")
}

/// The notification categories the app registers. An action exists only for a category the app registered, and the
/// extension names the category of a notice it opened (`NoticeCategory`).
public enum PushCategories {
    public static let replyAction = "hide.reply.action"

    public static func all() -> Set<UNNotificationCategory> {
        let reply = UNTextInputNotificationAction(
            identifier: replyAction,
            title: NSLocalizedString("push.action.reply", comment: "Reply button of a question notification"),
            // Face ID or the passcode first: the computer's connection secret is readable only while the phone is unlocked.
            options: [.authenticationRequired],
            textInputButtonTitle: NSLocalizedString("push.action.send", comment: "Send button of the reply field"),
            textInputPlaceholder: NSLocalizedString("push.action.placeholder", comment: "Placeholder of the reply field")
        )
        return [
            UNNotificationCategory(identifier: NoticeCategory.reply, actions: [reply], intentIdentifiers: [], options: []),
            UNNotificationCategory(identifier: NoticeCategory.open, actions: [], intentIdentifiers: [], options: []),
        ]
    }
}

/// The app's notification center delegate: shows a notification while the app is open, and hands a tap to the page.
/// An action is handled by the same delegate in a later step (`PushActions`).
public final class PushCenter: NSObject, UNUserNotificationCenterDelegate {
    public static let shared = PushCenter()
    public var inbox: PushOpenInbox = .shared
    /// Set by the app when it can send a reply without opening (`PushActions`).
    public var actions: PushActionHandling?

    public func install(center: UNUserNotificationCenter = .current()) {
        center.delegate = self
        center.setNotificationCategories(PushCategories.all())
    }

    public func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                       withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .list])
    }

    public func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                       withCompletionHandler completionHandler: @escaping () -> Void) {
        let info = response.notification.request.content.userInfo
        switch response.actionIdentifier {
        case UNNotificationDefaultActionIdentifier:
            inbox.offer(info)
            completionHandler()
        case PushCategories.replyAction:
            guard let text = (response as? UNTextInputNotificationResponse)?.userText, let actions else {
                completionHandler()
                return
            }
            actions.reply(text: text, notification: response.notification, completion: completionHandler)
        default:
            completionHandler()
        }
    }
}

/// What runs when a person answers from a notification without opening the app.
public protocol PushActionHandling: AnyObject {
    func reply(text: String, notification: UNNotification, completion: @escaping () -> Void)
}
