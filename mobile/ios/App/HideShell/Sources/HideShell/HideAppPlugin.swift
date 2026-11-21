import Capacitor
import UIKit

/// What the web app asks of the app itself: its version, the pairing link that opened it, and the light or dark the
/// page shows (which the status bar, the keyboard and the system's own colors follow).
@objc(HideAppPlugin)
open class HideAppPlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "HideAppPlugin"
    public let jsName = "HideApp"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "info", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "takeLink", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "appearance", returnType: CAPPluginReturnPromise),
    ]

    private let inbox: PairLinkInbox
    private var observer: NSObjectProtocol?

    public override init() {
        inbox = .shared
        super.init()
    }

    public init(inbox: PairLinkInbox) {
        self.inbox = inbox
        super.init()
    }

    open override func load() {
        // The event carries no link: the page asks for it with `takeLink`, so a secret never travels in an event.
        observer = NotificationCenter.default.addObserver(forName: .hidePairLinkArrived, object: inbox, queue: .main) { [weak self] _ in
            self?.notifyListeners("link", data: [:])
        }
    }

    deinit {
        if let observer { NotificationCenter.default.removeObserver(observer) }
    }

    @objc func info(_ call: CAPPluginCall) {
        let bundle = Bundle.main
        call.resolve([
            "version": bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "",
            "build": bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "",
        ])
    }

    @objc func takeLink(_ call: CAPPluginCall) {
        if let link = inbox.take() { call.resolve(["link": link]) } else { call.resolve([:]) }
    }

    @objc func appearance(_ call: CAPPluginCall) {
        let style: UIUserInterfaceStyle
        switch call.getString("scheme") {
        case "dark": style = .dark
        case "light": style = .light
        case "system": style = .unspecified
        default: call.reject("invalid_scheme", "invalid_scheme"); return
        }
        DispatchQueue.main.async { [weak self] in
            self?.bridge?.viewController?.overrideUserInterfaceStyle = style
            call.resolve()
        }
    }
}
