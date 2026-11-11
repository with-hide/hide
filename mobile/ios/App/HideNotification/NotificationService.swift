import UserNotifications
import HidePushCore

/// Replaces the fixed text of a push with the notice it carries, when this phone holds the key of the computer that
/// sent it. It has no network and no web view: it reads the Keychain items of the shared group, decrypts and sets the
/// title, the body, the category and what a tap needs. In every other case, and when the time it was given ends, the
/// fixed text stays as Apple delivered it and the notification opens nothing.
final class NotificationService: UNNotificationServiceExtension {
    private var handler: ((UNNotificationContent) -> Void)?
    private var delivered = false
    /// What Apple delivered, which stays the notification whenever it is not opened.
    private var fixed: UNNotificationContent?

    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        handler = contentHandler
        let fixed = request.content
        self.fixed = fixed
        guard let content = fixed.mutableCopy() as? UNMutableNotificationContent,
              let hide = fixed.userInfo["hide"] as? String,
              let keys = try? PushKeychain(accessGroup: PushKeychain.sharedAccessGroup()).all(),
              let notice = NoticeOpener.open(hide: hide, keys: keys) else {
            finish(fixed, forgetCiphertext: true)
            return
        }
        content.title = notice.title
        content.body = notice.body(words: .current)
        content.categoryIdentifier = NoticeCategory.of(notice)
        content.threadIdentifier = notice.tag
        content.userInfo = NoticeInfo.of(notice)
        finish(content, forgetCiphertext: false)
    }

    override func serviceExtensionTimeWillExpire() {
        if let fixed { finish(fixed, forgetCiphertext: true) }
    }

    private func finish(_ content: UNNotificationContent, forgetCiphertext: Bool) {
        guard !delivered else { return }
        delivered = true
        if forgetCiphertext, let mutable = content.mutableCopy() as? UNMutableNotificationContent {
            // The ciphertext is of no use to anything past this point, and a tap on the fixed text opens no agent.
            mutable.userInfo = [:]
            handler?(mutable)
        } else {
            handler?(content)
        }
    }
}
