import Foundation
import WebKit
import os

/// The only thing that may reach Capacitor's bridge from the web view.
///
/// Capacitor registers one script message handler, `bridge`, on the web view. WebKit exposes it to every frame
/// of the page, and Capacitor's handler never asks which frame or origin a call came from, nor which plugin it
/// names (it loads any plugin class by name). The phone app draws agent-made HTML in a sandboxed frame
/// (`look-frame.html`), so a script an agent wrote could call a native plugin. The gate takes the handler's place
/// under the same name and passes a call on only when it comes from the main frame of the app's own origin, is
/// a plugin call or an error report, and names a plugin the app has chosen to expose.
public final class BridgeGate: NSObject, WKScriptMessageHandler {
    public static let handlerName = "bridge"

    /// The origin the app serves its bundle from: Capacitor's `capacitor://localhost`.
    public struct Origin: Equatable {
        public let scheme: String
        public let host: String
        public init(scheme: String, host: String) { self.scheme = scheme; self.host = host }
        public static let app = Origin(scheme: "capacitor", host: "localhost")
    }

    /// The plugins the web side may call: Capacitor's own console, and the three of the app's own (their `jsName`).
    /// A plugin is added here in the same change that links it; `mobile/scripts/check-versions.mjs` compares this
    /// list with the plugin classes in this package and fails when they differ.
    public static let appPlugins: Set<String> = ["Console", "HideKeychain", "HideScanner", "HideApp"]

    public enum Verdict: Equatable {
        case pass
        case refuse(String)
    }

    private let inner: WKScriptMessageHandler
    private let origin: Origin
    private let plugins: Set<String>
    private let log = Logger(subsystem: "me.grab.hide.ios", category: "bridge")

    public init(inner: WKScriptMessageHandler, origin: Origin = .app, plugins: Set<String> = BridgeGate.appPlugins) {
        self.inner = inner
        self.origin = origin
        self.plugins = plugins
    }

    /// Replaces the handler registered under `bridge` with a gate in front of it.
    @discardableResult
    public static func install(on controller: WKUserContentController, inner: WKScriptMessageHandler,
                               origin: Origin = .app, plugins: Set<String> = BridgeGate.appPlugins) -> BridgeGate {
        let gate = BridgeGate(inner: inner, origin: origin, plugins: plugins)
        controller.removeScriptMessageHandler(forName: handlerName)
        controller.add(gate, name: handlerName)
        return gate
    }

    /// The decision, apart from WebKit so a test can state every case.
    public func verdict(isMainFrame: Bool, scheme: String, host: String, body: Any) -> Verdict {
        guard isMainFrame else { return .refuse("not_main_frame") }
        guard scheme == origin.scheme, host == origin.host else { return .refuse("foreign_origin") }
        guard let call = body as? [String: Any], let type = call["type"] as? String else { return .refuse("malformed") }
        switch type {
        case "js.error":
            return .pass
        case "message":
            guard let plugin = call["pluginId"] as? String, plugins.contains(plugin) else { return .refuse("plugin_not_exposed") }
            return .pass
        default:
            // Includes `cordova`: the app links no Cordova plugin.
            return .refuse("call_type")
        }
    }

    public func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        let origin = message.frameInfo.securityOrigin
        switch verdict(isMainFrame: message.frameInfo.isMainFrame, scheme: origin.protocol, host: origin.host, body: message.body) {
        case .pass:
            inner.userContentController(userContentController, didReceive: message)
        case .refuse(let reason):
            // The reason and the frame only; never the call's body, which can carry a credential.
            log.error("bridge call refused: \(reason, privacy: .public) mainFrame=\(message.frameInfo.isMainFrame, privacy: .public)")
        }
    }
}
