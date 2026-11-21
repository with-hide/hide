import Foundation

/// What the notification extension does, apart from UNNotificationServiceExtension so a test can state every case:
/// the fixed text stays unless one stored key opens the body for a computer that is still connected.
public enum NoticeOpener {
    /// `hide` is the base64url ciphertext APNs delivered next to the fixed alert.
    public static func open(hide: String, keys: [PushKey]) -> Notice? {
        guard let body = Base64URL.decode(hide) else { return nil }
        for key in keys {
            guard let plain = try? NotificationCipher.decrypt(body, privateKey: key.privateKey, auth: key.auth) else { continue }
            // The key that opened it belongs to one computer; a notice that names another room is not that computer's.
            guard let notice = Notice(json: plain), notice.room == key.room else { return nil }
            return notice
        }
        return nil
    }
}
