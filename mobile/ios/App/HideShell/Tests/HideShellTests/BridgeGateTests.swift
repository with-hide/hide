import WebKit
import XCTest
@testable import HideShell

/// Records what reaches it, in place of Capacitor's handler.
private final class Recorder: NSObject, WKScriptMessageHandler {
    private(set) var calls: [String] = []
    var onCall: ((String) -> Void)?
    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let body = message.body as? [String: Any], let id = body["callbackId"] as? String else { return }
        calls.append(id)
        onCall?(id)
    }
}

/// Serves a page of the app's own origin, `capacitor://localhost`, the way Capacitor's asset handler does.
private final class AppScheme: NSObject, WKURLSchemeHandler {
    // What the phone app does with agent-made HTML: a sandboxed frame of its own scheme. The same-origin frame
    // is the worse case, a frame that is not sandboxed at all.
    static let call = "webkit.messageHandlers.bridge.postMessage({type:'message',pluginId:'Console',methodName:'log',callbackId:'%@',options:{}})"
    static let pages: [String: String] = [
        "/parent.html": """
        <!doctype html><body><script>
        \(String(format: call, "main"))
        let done = 0;
        addEventListener('message', () => { if (++done === 2) setTimeout(() => { \(String(format: call, "main-final")) }, 400); });
        </script>
        <iframe sandbox="allow-scripts" src="/sandboxed.html"></iframe>
        <iframe src="/plain.html"></iframe></body>
        """,
        "/sandboxed.html": "<!doctype html><script>\(String(format: call, "sandboxed-frame")); parent.postMessage('done', '*')</script>",
        "/plain.html": "<!doctype html><script>\(String(format: call, "same-origin-frame")); parent.postMessage('done', '*')</script>",
    ]

    func webView(_ webView: WKWebView, start task: any WKURLSchemeTask) {
        let path = task.request.url?.path ?? ""
        guard let page = Self.pages[path], let url = task.request.url else {
            task.didFailWithError(URLError(.fileDoesNotExist))
            return
        }
        let response = URLResponse(url: url, mimeType: "text/html", expectedContentLength: -1, textEncodingName: "utf-8")
        task.didReceive(response)
        task.didReceive(Data(page.utf8))
        task.didFinish()
    }

    func webView(_ webView: WKWebView, stop task: any WKURLSchemeTask) {}
}

final class BridgeGateTests: XCTestCase {
    private var retained: [AnyObject] = []

    /// Loads the page with `bridge` handled by `attach` and returns what reached the recorder once the page is done.
    private func run(attach: (WKUserContentController, Recorder) -> Void) throws -> [String] {
        let recorder = Recorder()
        let configuration = WKWebViewConfiguration()
        configuration.setURLSchemeHandler(AppScheme(), forURLScheme: "capacitor")
        attach(configuration.userContentController, recorder)
        let webView = WKWebView(frame: CGRect(x: 0, y: 0, width: 320, height: 480), configuration: configuration)
        let window = UIWindow(frame: webView.frame)
        window.addSubview(webView)
        window.isHidden = false
        retained = [webView, window, recorder]
        let finished = expectation(description: "page ran to its last call")
        recorder.onCall = { if $0 == "main-final" { finished.fulfill() } }
        webView.load(URLRequest(url: URL(string: "capacitor://localhost/parent.html")!))
        wait(for: [finished], timeout: 20)
        return recorder.calls
    }

    /// Without the gate the same page reaches the handler from both frames. If this stops being true the app's own
    /// test (ShellViewControllerTests) no longer proves anything, because the premise of the gate (WebKit gives a
    /// frame the handler) is gone.
    func testWithoutTheGateFramesReachTheHandler() throws {
        let calls = try run { controller, recorder in controller.add(recorder, name: BridgeGate.handlerName) }
        XCTAssertEqual(calls.sorted(), ["main", "main-final", "same-origin-frame", "sandboxed-frame"])
    }

    func testVerdicts() {
        let gate = BridgeGate(inner: Recorder(), plugins: ["Console"])
        let call: [String: Any] = ["type": "message", "pluginId": "Console", "methodName": "log", "callbackId": "1", "options": [:]]
        func verdict(main: Bool = true, scheme: String = "capacitor", host: String = "localhost", _ body: Any) -> BridgeGate.Verdict {
            gate.verdict(isMainFrame: main, scheme: scheme, host: host, body: body)
        }
        XCTAssertEqual(verdict(call), .pass)
        XCTAssertEqual(verdict(["type": "js.error", "error": [:]]), .pass)
        XCTAssertEqual(verdict(main: false, call), .refuse("not_main_frame"))
        XCTAssertEqual(verdict(scheme: "https", host: "app.withhide.dev", call), .refuse("foreign_origin"))
        XCTAssertEqual(verdict(scheme: "capacitor", host: "evil.example", call), .refuse("foreign_origin"))
        XCTAssertEqual(verdict(scheme: "", host: "", call), .refuse("foreign_origin"))
        XCTAssertEqual(verdict(["type": "message", "pluginId": "CapacitorHttp", "methodName": "request", "callbackId": "1"]), .refuse("plugin_not_exposed"))
        XCTAssertEqual(verdict(["type": "message", "methodName": "log"]), .refuse("plugin_not_exposed"))
        XCTAssertEqual(verdict(["type": "cordova", "service": "Console", "action": "log"]), .refuse("call_type"))
        XCTAssertEqual(verdict("not a dictionary"), .refuse("malformed"))
        XCTAssertEqual(verdict([:]), .refuse("malformed"))
    }

    /// The list a page may call is Capacitor's console and the app's own three plugins, by the names their classes carry.
    func testTheShippedPluginListIsTheConsoleAndTheAppsOwnPlugins() {
        let own = [HideKeychainPlugin().jsName, HideScannerPlugin().jsName, HideAppPlugin().jsName]
        XCTAssertEqual(BridgeGate.appPlugins, Set(["Console"] + own))
    }

    /// A frame that is not the app's main frame reaches none of the app's plugins, and neither does a foreign origin.
    func testNoFrameButTheMainFrameReachesAnAppPlugin() {
        let gate = BridgeGate(inner: Recorder())
        for plugin in ["HideKeychain", "HideScanner", "HideApp"] {
            let call: [String: Any] = ["type": "message", "pluginId": plugin, "methodName": "list", "callbackId": "1", "options": [:]]
            XCTAssertEqual(gate.verdict(isMainFrame: false, scheme: "capacitor", host: "localhost", body: call), .refuse("not_main_frame"), plugin)
            XCTAssertEqual(gate.verdict(isMainFrame: true, scheme: "https", host: "app.withhide.dev", body: call), .refuse("foreign_origin"), plugin)
            XCTAssertEqual(gate.verdict(isMainFrame: true, scheme: "capacitor", host: "localhost", body: call), .pass, plugin)
        }
    }
}

final class NavigationPolicyTests: XCTestCase {
    func testOnlyTheAppAndHttpsAreAllowed() {
        let policy = NavigationPolicy(inner: Dummy())
        for allowed in ["capacitor://localhost/p/5/index.html", "https://github.com/with-hide/hide", "about:blank"] {
            XCTAssertTrue(policy.allows(URL(string: allowed)), allowed)
        }
        // Only https goes outside the app, by a navigation or by a new window.
        XCTAssertTrue(policy.opensOutside(URL(string: "https://github.com")))
        for inside in ["capacitor://localhost/", "about:blank", "tel:1", "http://example.com"] {
            XCTAssertFalse(policy.opensOutside(URL(string: inside)), inside)
        }
        for refused in ["tel:+821012345678", "sms:1", "http://example.com", "capacitor://other/x", "file:///etc/passwd",
                        "data:text/html,hi", "javascript:alert(1)", "itms-apps://apps.apple.com/app/x", "hide://pair"] {
            XCTAssertFalse(policy.allows(URL(string: refused)), refused)
        }
        XCTAssertFalse(policy.allows(nil))
    }
}

private final class Dummy: NSObject, WKNavigationDelegate, WKUIDelegate {}

final class PhoneRouterTests: XCTestCase {
    func testProtocolBundlesAreTheirOwnPagesAndEverythingElseKeepsCapacitorsRule() {
        var router = PhoneRouter()
        router.basePath = "/app"
        XCTAssertEqual(router.route(for: "/p/5/"), "/app/p/5/index.html")
        XCTAssertEqual(router.route(for: "/p/12"), "/app/p/12/index.html")
        XCTAssertEqual(router.route(for: "/"), "/app/index.html")
        XCTAssertEqual(router.route(for: "/p/5/assets/phone-bundle-abc.js"), "/app/p/5/assets/phone-bundle-abc.js")
        XCTAssertEqual(router.route(for: "/p/5/look-frame.html"), "/app/p/5/look-frame.html")
        XCTAssertEqual(router.route(for: "/assets/relay_bg.wasm"), "/app/assets/relay_bg.wasm")
        // Not a protocol folder: the loader answers, as for any address of a page.
        XCTAssertEqual(router.route(for: "/p/05/"), "/app/index.html")
        XCTAssertEqual(router.route(for: "/p/5/x/"), "/app/index.html")
        XCTAssertEqual(router.route(for: "/p/"), "/app/index.html")
    }
}
