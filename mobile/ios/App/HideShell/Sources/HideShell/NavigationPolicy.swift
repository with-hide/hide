import Foundation
import WebKit

/// Where the web view may go. Capacitor hands every top-level address outside the app to the system
/// (`UIApplication.open`), whatever its scheme, so a page could open `tel:` or another app's scheme. The policy
/// lets a navigation inside the app's origin through to Capacitor and opens only `https` outside it; any other
/// scheme is cancelled.
public final class NavigationPolicy: NSObject, WKNavigationDelegate {
    private let inner: WKNavigationDelegate?
    private let origin: BridgeGate.Origin

    public init(inner: WKNavigationDelegate, origin: BridgeGate.Origin = .app) {
        self.inner = inner
        self.origin = origin
    }

    /// True when a navigation to `url` may go on to Capacitor's own delegate.
    public func allows(_ url: URL?) -> Bool {
        guard let url, let scheme = url.scheme?.lowercased() else { return false }
        if scheme == origin.scheme { return url.host == origin.host }
        // `about:blank` and `blob:` frames the page makes of itself stay inside it.
        if scheme == "about" || scheme == "blob" { return true }
        return scheme == "https"
    }

    public func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
        guard allows(navigationAction.request.url) else {
            decisionHandler(.cancel)
            return
        }
        // Capacitor's delegate always answers; with it gone there is no one to hand the address to.
        guard let inner else {
            decisionHandler(.cancel)
            return
        }
        inner.webView?(webView, decidePolicyFor: navigationAction, decisionHandler: decisionHandler)
    }

    // The rest of the delegate is Capacitor's.
    public override func responds(to aSelector: Selector!) -> Bool {
        super.responds(to: aSelector) || (inner?.responds(to: aSelector) ?? false)
    }

    public override func forwardingTarget(for aSelector: Selector!) -> Any? {
        inner?.responds(to: aSelector) == true ? inner : super.forwardingTarget(for: aSelector)
    }
}
