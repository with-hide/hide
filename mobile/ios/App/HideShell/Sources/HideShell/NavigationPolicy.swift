import Foundation
import WebKit

/// Where the web view may go, decided in one place for both ways a page can leave: a navigation, and a new window
/// (`window.open`, a link with a target).
///
/// Capacitor hands every top-level address outside the app to the system (`UIApplication.open`), whatever its
/// scheme, so a page could open `tel:` or another app's scheme. The policy lets a navigation inside the app's origin
/// through to Capacitor and lets only an `https` address go outside it; any other scheme is cancelled. A new window
/// is never made: its address goes to Capacitor, which opens it outside, only when it is `https`.
public final class NavigationPolicy: NSObject, WKNavigationDelegate, WKUIDelegate {
    private let inner: WKNavigationDelegate & WKUIDelegate
    private let origin: BridgeGate.Origin

    public init(inner: WKNavigationDelegate & WKUIDelegate, origin: BridgeGate.Origin = .app) {
        self.inner = inner
        self.origin = origin
    }

    /// True when `url` may be opened outside the app, which only `https` may.
    public func opensOutside(_ url: URL?) -> Bool {
        url?.scheme?.lowercased() == "https"
    }

    /// True when a navigation to `url` may go on to Capacitor's own delegate.
    public func allows(_ url: URL?) -> Bool {
        guard let url, let scheme = url.scheme?.lowercased() else { return false }
        if scheme == origin.scheme { return url.host == origin.host }
        // `about:blank` and `blob:` frames the page makes of itself stay inside it.
        if scheme == "about" || scheme == "blob" { return true }
        return opensOutside(url)
    }

    public func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
        guard allows(navigationAction.request.url) else {
            decisionHandler(.cancel)
            return
        }
        // A delegate that does not decide leaves WebKit's default, which is to go on.
        if inner.webView?(webView, decidePolicyFor: navigationAction, decisionHandler: decisionHandler) == nil {
            decisionHandler(.allow)
        }
    }

    public func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                        for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        guard opensOutside(navigationAction.request.url) else { return nil }
        return inner.webView?(webView, createWebViewWith: configuration, for: navigationAction, windowFeatures: windowFeatures) ?? nil
    }

    // The rest of both delegates is Capacitor's.
    public override func responds(to aSelector: Selector!) -> Bool {
        super.responds(to: aSelector) || inner.responds(to: aSelector)
    }

    public override func forwardingTarget(for aSelector: Selector!) -> Any? {
        inner.responds(to: aSelector) ? inner : super.forwardingTarget(for: aSelector)
    }
}
