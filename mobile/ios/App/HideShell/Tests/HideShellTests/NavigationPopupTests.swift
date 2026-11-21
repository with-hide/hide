import UIKit
import WebKit
import XCTest
@testable import HideShell

/// Records the address of every new window the page asks for, and answers none.
private final class Opener: NSObject, WKNavigationDelegate, WKUIDelegate {
    private(set) var asked: [String] = []
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        asked.append(navigationAction.request.url?.absoluteString ?? "")
        return nil
    }
}

private final class Done: NSObject, WKScriptMessageHandler {
    var onDone: (() -> Void)?
    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) { onDone?() }
}

private final class Page: NSObject, WKURLSchemeHandler {
    func webView(_ webView: WKWebView, start task: any WKURLSchemeTask) {
        let page = """
        <!doctype html><script>
        window.open('tel:+821012345678');
        window.open('capacitor://localhost/elsewhere.html');
        window.open('http://example.com/plain');
        window.open('https://example.com/secure');
        setTimeout(() => webkit.messageHandlers.done.postMessage(1), 500);
        </script>
        """
        let url = task.request.url!
        task.didReceive(URLResponse(url: url, mimeType: "text/html", expectedContentLength: -1, textEncodingName: "utf-8"))
        task.didReceive(Data(page.utf8))
        task.didFinish()
    }
    func webView(_ webView: WKWebView, stop task: any WKURLSchemeTask) {}
}

final class NavigationPopupTests: XCTestCase {
    func testANewWindowOpensOutsideOnlyForHttps() {
        let opener = Opener()
        let done = Done()
        let configuration = WKWebViewConfiguration()
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = true
        configuration.setURLSchemeHandler(Page(), forURLScheme: "capacitor")
        configuration.userContentController.add(done, name: "done")
        let webView = WKWebView(frame: CGRect(x: 0, y: 0, width: 320, height: 480), configuration: configuration)
        let policy = NavigationPolicy(inner: opener)
        webView.navigationDelegate = policy
        webView.uiDelegate = policy
        let window = UIWindow(frame: webView.frame)
        window.addSubview(webView)
        window.isHidden = false
        let finished = expectation(description: "page asked for its windows")
        done.onDone = { finished.fulfill() }
        webView.load(URLRequest(url: URL(string: "capacitor://localhost/index.html")!))
        wait(for: [finished], timeout: 20)
        XCTAssertEqual(opener.asked, ["https://example.com/secure"])
        withExtendedLifetime((window, webView, policy, done)) {}
    }
}
