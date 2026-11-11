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
        lock.lock()
        target = Target(room: room, deviceId: device, paneId: pane)
        lock.unlock()
        // After the lock: an observer that calls `take()` from the notification must not wait on it.
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

/// New notices open the conversation. The old question category is registered without actions too, so a
/// notification delivered by an earlier build cannot keep offering Reply after this app starts.
public enum PushCategories {
    public static func all() -> Set<UNNotificationCategory> {
        Set([NoticeCategory.open, "hide.reply"].map {
            UNNotificationCategory(identifier: $0, actions: [], intentIdentifiers: [], options: [])
        })
    }
}

/// The app's notification center delegate: shows a notification while the app is open, and hands a tap to the page.
public final class PushCenter: NSObject, UNUserNotificationCenterDelegate {
    public static let shared = PushCenter()
    public var inbox: PushOpenInbox = .shared

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
        default:
            completionHandler()
        }
    }
}
