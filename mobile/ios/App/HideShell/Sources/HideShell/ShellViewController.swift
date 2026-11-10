import Capacitor
import UIKit
import WebKit

/// Capacitor's view controller with the app's guards put in place once the bridge exists.
@objc(ShellViewController)
open class ShellViewController: CAPBridgeViewController {
    private var navigationPolicy: NavigationPolicy?

    /// The plugins a script of the app may call. A test names its own.
    open var exposedPlugins: Set<String> { BridgeGate.appPlugins }

    /// The app's own plugins, the ones `BridgeGate.appPlugins` names besides Capacitor's console. A test names its own.
    open func appPlugins() -> [CAPPlugin] {
        [HideKeychainPlugin(), HideScannerPlugin(), HideAppPlugin(), HidePushPlugin()]
    }

    /// The height the keyboard covers, which the web view gives up so the page's last line stays above it.
    private var keyboardOverlap: CGFloat = 0

    /// The phone site is several pages (the loader and one bundle per protocol), not one page.
    public override func router() -> Router {
        PhoneRouter()
    }

    open override func capacitorDidLoad() {
        guard let handler = (bridge as? CapacitorBridge)?.webViewDelegationHandler, let webView,
              webView.configuration.userContentController === handler.contentController else {
            // Without the handler on this very web view there is nothing to guard, and an unguarded bridge must not run.
            fatalError("Capacitor did not hand the web view the handler it registered the bridge on")
        }
        BridgeGate.install(on: handler.contentController, inner: handler, plugins: exposedPlugins)
        let policy = NavigationPolicy(inner: handler)
        navigationPolicy = policy
        webView.navigationDelegate = policy
        webView.uiDelegate = policy
        for plugin in appPlugins() { bridge?.registerPluginInstance(plugin) }
        feelNative(webView)
    }

    /// What makes the page stop feeling like a page: the whole page does not rubber-band or zoom (the lists inside it
    /// scroll as they do), and the web view ends where the keyboard begins so the reply field is never under it.
    private func feelNative(_ webView: WKWebView) {
        let scroll = webView.scrollView
        scroll.bounces = false
        scroll.alwaysBounceVertical = false
        scroll.alwaysBounceHorizontal = false
        scroll.minimumZoomScale = 1
        scroll.maximumZoomScale = 1
        scroll.pinchGestureRecognizer?.isEnabled = false
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardWillChange(_:)),
                                               name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardWillChange(_:)),
                                               name: UIResponder.keyboardWillHideNotification, object: nil)
    }

    @objc private func keyboardWillChange(_ note: Notification) {
        guard let webView, let end = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect else { return }
        let hiding = note.name == UIResponder.keyboardWillHideNotification
        keyboardOverlap = hiding ? 0 : max(0, view.bounds.maxY - view.convert(end, from: nil).minY)
        let duration = note.userInfo?[UIResponder.keyboardAnimationDurationUserInfoKey] as? Double ?? 0.25
        UIView.animate(withDuration: duration) {
            webView.frame = CGRect(x: 0, y: 0, width: self.view.bounds.width, height: self.view.bounds.height - self.keyboardOverlap)
        }
    }

    open override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        webView?.frame = CGRect(x: 0, y: 0, width: view.bounds.width, height: view.bounds.height - keyboardOverlap)
    }

    open override var supportedInterfaceOrientations: UIInterfaceOrientationMask { .portrait }

#if DEBUG
    /// The simulator check's inputs, in a debug build only: the simulator has no camera, cannot open a universal link
    /// and nothing outside can tap the web view. A pairing link given as the launch value `HIDE_DEBUG_PAIR_LINK` is
    /// handed to the same inbox a link opened from outside reaches, and Connect is pressed once the page shows it.
    /// `HIDE_DEBUG_TAPS` is selectors separated by `;;`, each pressed once the page has it, in order. A release build
    /// has no such way in.
    public override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        let environment = ProcessInfo.processInfo.environment
        if let link = environment["HIDE_DEBUG_PAIR_LINK"], URL(string: link) != nil {
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
                PairLinkInbox.shared.offer(URL(string: link))
                self?.pressConnect(attempt: 0)
            }
        }
        if let taps = environment["HIDE_DEBUG_TAPS"], !taps.isEmpty {
            let selectors = taps.components(separatedBy: ";;").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in self?.tap(selectors, attempt: 0) }
        }
    }

    private func tap(_ selectors: [String], attempt: Int) {
        guard let selector = selectors.first, attempt < 120,
              let literal = try? JSONEncoder().encode(selector), let source = String(data: literal, encoding: .utf8) else { return }
        let script = "(() => { const found = document.querySelector(\(source)); if (!found) return false; found.click(); return true; })()"
        webView?.evaluateJavaScript(script) { [weak self] result, _ in
            let pressed = result as? Bool == true
            DispatchQueue.main.asyncAfter(deadline: .now() + (pressed ? 1.5 : 0.5)) {
                self?.tap(pressed ? Array(selectors.dropFirst()) : selectors, attempt: pressed ? 0 : attempt + 1)
            }
        }
    }

    private func pressConnect(attempt: Int) {
        let script = """
        (() => {
          const button = [...document.querySelectorAll("button")].find((candidate) => candidate.textContent.trim() === "Connect");
          if (!button) return false;
          button.click();
          return true;
        })()
        """
        webView?.evaluateJavaScript(script) { [weak self] result, _ in
            guard result as? Bool != true, attempt < 120 else { return }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { self?.pressConnect(attempt: attempt + 1) }
        }
    }
#endif
}
