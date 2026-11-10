import Foundation

/// A Hide pairing link that arrived from outside the app (a QR scanned with the camera, a link in a message).
///
/// The pairing secret rides in the fragment, `https://app.withhide.dev/#pair=<base64url>` (docs/RELAY_PROTOCOL.md).
/// Native code only checks the shape, so a link of another form never reaches the web view at all; the web app
/// decodes it with the same decoder as a scanned QR, which is what validates it, and a person presses Connect.
public enum PairLink {
    public static let host = "app.withhide.dev"
    public static let maxBytes = 1024

    /// The link as a string when it is exactly a pairing link, nil for anything else.
    public static func accept(_ url: URL?) -> String? {
        guard let url, url.scheme == "https", url.host?.lowercased() == host, url.port == nil,
              url.user == nil, url.password == nil, url.query == nil,
              url.path.isEmpty || url.path == "/",
              let fragment = url.fragment, fragment.hasPrefix("pair="), fragment.count > "pair=".count else { return nil }
        let payload = fragment.dropFirst("pair=".count)
        guard payload.utf8.allSatisfy({ ($0 >= 0x30 && $0 <= 0x39) || ($0 >= 0x41 && $0 <= 0x5A) || ($0 >= 0x61 && $0 <= 0x7A) || $0 == 0x2D || $0 == 0x5F }) else { return nil }
        let text = url.absoluteString
        return text.utf8.count <= maxBytes ? text : nil
    }
}

public extension Notification.Name {
    /// Posted, with no secret in it, when a pairing link is waiting in `PairLinkInbox`.
    static let hidePairLinkArrived = Notification.Name("me.grab.hide.ios.pair-link-arrived")
}

/// The one link waiting for the web app, held in memory only and for one minute: the web side takes it once and a
/// retained older bundle that never asks cannot leave it for a later screen to find.
public final class PairLinkInbox {
    public static let shared = PairLinkInbox()
    public static let lifetime: TimeInterval = 60

    private let lock = NSLock()
    private var pending: (link: String, at: Date)?
    private let now: () -> Date

    public init(now: @escaping () -> Date = Date.init) {
        self.now = now
    }

    /// Keeps the link when it is a pairing link and tells the web side that one is waiting; false for any other.
    @discardableResult
    public func offer(_ url: URL?) -> Bool {
        guard let link = PairLink.accept(url) else { return false }
        lock.lock()
        pending = (link, now())
        lock.unlock()
        NotificationCenter.default.post(name: .hidePairLinkArrived, object: self)
        return true
    }

    /// The waiting link, once.
    public func take() -> String? {
        lock.lock(); defer { lock.unlock() }
        defer { pending = nil }
        guard let pending, now().timeIntervalSince(pending.at) <= Self.lifetime else { return nil }
        return pending.link
    }
}
