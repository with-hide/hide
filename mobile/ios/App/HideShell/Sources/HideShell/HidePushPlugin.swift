import Capacitor
import Foundation
import UserNotifications
import HidePushCore

/// The web app's way to the app's notifications: whether they are allowed, turning them on for one computer, and
/// removing a computer's key. There is no call that reads a private key, sends an action or takes a key of the caller's
/// own, and a frame that is not the app's main frame never reaches this class (`BridgeGate`). A tap on a notification
/// reaches the page as `takeOpen` and an action is handled natively (`PushActions`), never by the page.
@objc(HidePushPlugin)
open class HidePushPlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "HidePushPlugin"
    public let jsName = "HidePush"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "state", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "enable", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "registration", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "forget", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "takeOpen", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "demoSample", returnType: CAPPluginReturnPromise),
    ]

    private let keychain: PushKeychain
    private let token: PushToken
    private let inbox: PushOpenInbox
    private let center: () -> UNUserNotificationCenter
    private var observer: NSObjectProtocol?

    public override init() {
        keychain = PushKeychain(accessGroup: PushKeychain.sharedAccessGroup())
        token = .shared
        inbox = .shared
        center = { UNUserNotificationCenter.current() }
        super.init()
    }

    public init(keychain: PushKeychain, token: PushToken, inbox: PushOpenInbox, center: @escaping () -> UNUserNotificationCenter) {
        self.keychain = keychain
        self.token = token
        self.inbox = inbox
        self.center = center
        super.init()
    }

    open override func load() {
        // The event carries no target: the page asks for it with `takeOpen`.
        observer = NotificationCenter.default.addObserver(forName: .hidePushOpenArrived, object: inbox, queue: .main) { [weak self] _ in
            self?.notifyListeners("open", data: [:])
        }
    }

    deinit {
        if let observer { NotificationCenter.default.removeObserver(observer) }
    }

    private func fail(_ call: CAPPluginCall, _ code: String) { call.reject(code, code) }

    private func room(_ call: CAPPluginCall) -> String? {
        guard let room = call.getString("room"), PushKeychain.isRoom(room) else { fail(call, "invalid_room"); return nil }
        return room
    }

    /// `unknown` before the person has been asked, `denied` after a refusal, `granted` otherwise.
    static func permission(_ settings: UNNotificationSettings) -> String {
        switch settings.authorizationStatus {
        case .notDetermined: return "unknown"
        case .denied: return "denied"
        default: return "granted"
        }
    }

    @objc func state(_ call: CAPPluginCall) {
        center().getNotificationSettings { call.resolve(["permission": Self.permission($0)]) }
    }

    /// Turns notifications on for one computer: asks the system for permission only now (this is called from the person's
    /// tap), registers with Apple, makes the computer's key and answers with what the computer needs to encrypt for it.
    @objc func enable(_ call: CAPPluginCall) {
        guard let room = room(call) else { return }
        let center = center()
        center.getNotificationSettings { [weak self] settings in
            guard let self else { return }
            let proceed = { (granted: Bool) in
                guard granted else { self.fail(call, "denied"); return }
                self.registration(room: room, create: true, call: call)
            }
            switch settings.authorizationStatus {
            case .notDetermined:
                center.requestAuthorization(options: [.alert, .sound]) { granted, _ in proceed(granted) }
            case .denied:
                proceed(false)
            default:
                proceed(true)
            }
        }
    }

    /// The registration to send again on a start or when Apple changed the token: only for a computer whose key exists and
    /// only while the system still allows notifications. Nothing is created here.
    @objc func registration(_ call: CAPPluginCall) {
        guard let room = room(call) else { return }
        center().getNotificationSettings { [weak self] settings in
            guard let self, Self.permission(settings) == "granted", (try? self.keychain.read(room: room)) != nil else {
                call.resolve(["found": false])
                return
            }
            self.registration(room: room, create: false, call: call)
        }
    }

    private func registration(room: String, create: Bool, call: CAPPluginCall) {
        token.request { [weak self] result in
            guard let self else { return }
            switch result {
            case .failure(let failure):
                self.fail(call, failure == .timedOut ? "token_timeout" : "token_unavailable")
            case .success(let hex):
                do {
                    guard let key = try (create ? self.keychain.read(room: room) ?? self.keychain.create(room: room) : self.keychain.read(room: room)) else {
                        call.resolve(["found": false])
                        return
                    }
                    call.resolve([
                        "found": true, "token": hex, "environment": PushEnvironment.current,
                        "p256dh": Base64URL.encode(key.publicKey), "auth": Base64URL.encode(key.auth),
                    ])
                } catch { self.fail(call, (error as? PushKeychainError)?.code ?? "keychain_failed") }
            }
        }
    }

    /// Deletes a computer's key: its notices open no more. Called after the unregister was sent, or without it when the
    /// computer could not be reached.
    @objc func forget(_ call: CAPPluginCall) {
        guard let room = room(call) else { return }
        do { try keychain.remove(room: room); call.resolve() }
        catch { fail(call, (error as? PushKeychainError)?.code ?? "keychain_failed") }
    }

    @objc func takeOpen(_ call: CAPPluginCall) {
        guard let target = inbox.take() else { call.resolve([:]); return }
        call.resolve(["room": target.room, "device_id": target.deviceId, "pane_id": target.paneId])
    }

    /// A local notification for the demo mode: no token, no key, no network. It uses the same category as a real question,
    /// so the Reply button is there; its action does nothing a real one would (`demo` in its user info).
    @objc func demoSample(_ call: CAPPluginCall) {
        let center = center()
        let words = call.getObject("words") ?? [:]
        let send = { (granted: Bool) in
            guard granted else { self.fail(call, "denied"); return }
            let content = UNMutableNotificationContent()
            content.title = (words["title"] as? String).map { String($0.prefix(200)) } ?? ""
            content.body = (words["body"] as? String).map { String($0.prefix(400)) } ?? ""
            content.categoryIdentifier = NoticeCategory.reply
            content.userInfo = ["demo": true]
            let request = UNNotificationRequest(identifier: "hide.demo.sample", content: content, trigger: UNTimeIntervalNotificationTrigger(timeInterval: 2, repeats: false))
            center.add(request) { error in
                if error == nil { call.resolve() } else { self.fail(call, "notification_failed") }
            }
        }
        center.getNotificationSettings { settings in
            switch settings.authorizationStatus {
            case .notDetermined: center.requestAuthorization(options: [.alert, .sound]) { granted, _ in send(granted) }
            case .denied: send(false)
            default: send(true)
            }
        }
    }
}
