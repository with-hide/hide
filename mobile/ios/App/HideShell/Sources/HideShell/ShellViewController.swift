import Capacitor
import UIKit
import WebKit

/// Capacitor's view controller with the app's guards put in place once the bridge exists.
@objc(ShellViewController)
open class ShellViewController: CAPBridgeViewController, UIGestureRecognizerDelegate {
    private var navigationPolicy: NavigationPolicy?

    /// The plugins a script of the app may call. A test names its own.
    open var exposedPlugins: Set<String> { BridgeGate.appPlugins }

    /// The app's own plugins, the ones `BridgeGate.appPlugins` names besides Capacitor's console. A test names its own.
    open func appPlugins() -> [CAPPlugin] {
        [HideKeychainPlugin(), HideScannerPlugin(), HideAppPlugin(), HidePushPlugin()]
    }

    /// Where the keyboard ends up, in the window's coordinates; nil while it is hidden.
    private var keyboardEnd: CGRect?

    /// The web view's frame in its window: all of it, less what the keyboard covers at the bottom, so the page's last
    /// line stays above the keyboard. It is measured from the window and never from the web view's own bounds: Capacitor
    /// makes the web view the controller's view, so a frame taken from those bounds would shrink a view that had already
    /// shrunk, on every layout pass, and would not come back when the keyboard hides.
    public static func webFrame(in window: CGRect, keyboard: CGRect?) -> CGRect {
        guard let keyboard else { return window }
        let covered = max(0, min(window.height, window.maxY - keyboard.minY))
        return CGRect(x: window.minX, y: window.minY, width: window.width, height: window.height - covered)
    }

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
#if DEBUG
        startClock(on: handler.contentController)
#endif
    }

    /// What makes the page stop feeling like a page: the whole page does not rubber-band or zoom (the lists inside it
    /// scroll as they do), and the web view ends where the keyboard begins so the reply field is never under it.
    private func feelNative(_ webView: WKWebView) {
        // Keep the launch surface through WebKit's first paint and the loader-to-bundle navigation.
        webView.isOpaque = false
        webView.backgroundColor = UIColor(named: "LaunchBackground")
        webView.underPageBackgroundColor = UIColor(named: "LaunchBackground") ?? .systemBackground
        let scroll = webView.scrollView
        scroll.backgroundColor = .clear
        // Styles can paint before the module that calls followPhoneTheme. Seed that same stored choice at document
        // start, including each protocol navigation; settings.ts continues to own subsequent changes.
        webView.configuration.userContentController.addUserScript(WKUserScript(source: """
        (() => {
          let theme;
          try { theme = JSON.parse(localStorage.getItem('hide.phone.settings'))?.theme; } catch {}
          const dark = theme === 'dark' || (theme !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches);
          const apply = () => {
            if (!document.documentElement) return false;
            document.documentElement.classList.toggle('dark', dark);
            return true;
          };
          if (!apply()) {
            const observer = new MutationObserver(() => { if (apply()) observer.disconnect(); });
            observer.observe(document, { childList: true });
          }
        })();
        """, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        scroll.bounces = false
        scroll.alwaysBounceVertical = false
        scroll.alwaysBounceHorizontal = false
        scroll.minimumZoomScale = 1
        scroll.maximumZoomScale = 1
        scroll.pinchGestureRecognizer?.isEnabled = false
        // Detail screens are page state, not WebKit history. One edge recognizer presses the page's own back
        // control, so a mission, a View and the start pages keep their existing owner and action.
        let back = UIScreenEdgePanGestureRecognizer(target: self, action: #selector(swipeBack(_:)))
        back.edges = .left
        back.delegate = self
        webView.addGestureRecognizer(back)
        scroll.panGestureRecognizer.require(toFail: back)
        // The web view's own answer to the keyboard (an inset and a scroll of the page) would be made a second time on
        // top of the frame below, as Capacitor's keyboard plugin found: the frame alone makes room.
        let center = NotificationCenter.default
        for name in [UIResponder.keyboardWillHideNotification, UIResponder.keyboardWillShowNotification,
                     UIResponder.keyboardWillChangeFrameNotification, UIResponder.keyboardDidChangeFrameNotification] {
            center.removeObserver(webView, name: name, object: nil)
        }
        center.addObserver(self, selector: #selector(keyboardWillChange(_:)), name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        center.addObserver(self, selector: #selector(keyboardWillChange(_:)), name: UIResponder.keyboardWillHideNotification, object: nil)
    }

    public func gestureRecognizerShouldBegin(_ gesture: UIGestureRecognizer) -> Bool {
        guard let edge = gesture as? UIScreenEdgePanGestureRecognizer else { return true }
        let velocity = edge.velocity(in: webView)
        return velocity.x > abs(velocity.y)
    }

    @objc private func swipeBack(_ gesture: UIScreenEdgePanGestureRecognizer) {
        guard gesture.state == .ended, let webView,
              gesture.translation(in: webView).x > 0,
              gesture.translation(in: webView).x > webView.bounds.width / 2 || gesture.velocity(in: webView).x > webView.bounds.width else { return }
        webView.evaluateJavaScript("""
        (() => {
          const dialog = [...document.querySelectorAll('[role="dialog"][data-state="open"]')].pop();
          const back = (dialog || document).querySelector('button[data-phone-back]:not(:disabled)');
          if (!back || !back.getClientRects().length) return;
          document.activeElement?.blur();
          back.click();
        })()
        """, completionHandler: nil)
    }

    /// The window the web view fills; the screen before it has one.
    private var windowBounds: CGRect { view.window?.bounds ?? UIScreen.main.bounds }

    @objc private func keyboardWillChange(_ note: Notification) {
        guard let webView, let end = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect else { return }
        // The end frame is the screen's; an iPhone app's window is the screen.
        keyboardEnd = note.name == UIResponder.keyboardWillHideNotification ? nil : end
        let duration = note.userInfo?[UIResponder.keyboardAnimationDurationUserInfoKey] as? Double ?? 0.25
        UIView.animate(withDuration: duration) {
            webView.frame = Self.webFrame(in: self.windowBounds, keyboard: self.keyboardEnd)
        }
    }

    open override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        webView?.frame = Self.webFrame(in: windowBounds, keyboard: keyboardEnd)
    }

    open override var supportedInterfaceOrientations: UIInterfaceOrientationMask { .portrait }

#if DEBUG
    /// The simulator check's inputs, in a debug build only: the simulator has no camera, cannot open a universal link
    /// and nothing outside can tap the web view. A pairing link given as the launch value `HIDE_DEBUG_PAIR_LINK` is
    /// handed to the same inbox a link opened from outside reaches, and Connect is pressed once the page shows it.
    /// `HIDE_DEBUG_TAPS` is selectors separated by `;;`, each pressed once the page has it, in order; a selector written
    /// `<selector>::=<text>` is filled with that text instead of pressed, and one written `<selector>::^` is scrolled to the
    /// top instead. The file `hide-debug-taps` in the app's
    /// temporary folder says how it went: `done` once the last one is pressed, `failed <selector>` when one is never
    /// found, so a script that waits on it fails instead of capturing a screen the steps never reached. `HIDE_DEBUG_NOW` is a local date and time
    /// (`2026-11-09T09:41:00`) the page's clock starts from, so a capture shows the same times on every run. A release
    /// build has no such way in.
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

    /// Starts the page's clock at `HIDE_DEBUG_NOW` and lets it run on from there. It is put in before any page script runs.
    private func startClock(on controller: WKUserContentController) {
        guard let now = ProcessInfo.processInfo.environment["HIDE_DEBUG_NOW"], !now.isEmpty,
              let literal = try? JSONEncoder().encode(now), let source = String(data: literal, encoding: .utf8) else { return }
        let script = """
        (() => {
          const Real = Date, base = new Real(\(source)).getTime(), began = Real.now();
          if (Number.isNaN(base)) return;
          const now = () => base + (Real.now() - began);
          Date = new Proxy(Real, {
            construct: (target, args, newTarget) => Reflect.construct(target, args.length === 0 ? [now()] : args, newTarget),
            get: (target, key, receiver) => (key === 'now' ? now : Reflect.get(target, key, receiver)),
          });
        })();
        """
        controller.addUserScript(WKUserScript(source: script, injectionTime: .atDocumentStart, forMainFrameOnly: false))
    }

    private func tap(_ selectors: [String], attempt: Int) {
        guard let step = selectors.first else { return }
        guard attempt < 120 else { reportTaps("failed \(step)"); return }
        // `<selector>::^` brings that element to the top of what scrolls it instead of pressing it.
        let scrolls = step.hasSuffix("::^")
        let parts = (scrolls ? String(step.dropLast(3)) : step).components(separatedBy: "::=")
        let selector = parts[0].trimmingCharacters(in: .whitespaces)
        guard let literal = try? JSONEncoder().encode(selector), let source = String(data: literal, encoding: .utf8) else { return }
        // A press is a mouse-down and a click: the page's tab strip reacts to the first, its buttons to the second.
        var action = "found.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true, button: 0 })); found.click();"
        if scrolls {
            action = "found.scrollIntoView({ block: 'start' });"
        } else if parts.count > 1 {
            // A field of the page is filled the way typing does: through the element's own setter, then an input event, so the page's state sees it.
            guard let text = try? JSONEncoder().encode(parts.dropFirst().joined(separator: "::=")), let typed = String(data: text, encoding: .utf8) else { return }
            action = "Object.getOwnPropertyDescriptor(Object.getPrototypeOf(found), 'value').set.call(found, \(typed)); found.dispatchEvent(new Event('input', { bubbles: true }));"
        }
        let script = "(() => { const found = document.querySelector(\(source)); if (!found) return false; \(action) return true; })()"
        webView?.evaluateJavaScript(script) { [weak self] result, _ in
            let pressed = result as? Bool == true
            if pressed && selectors.count == 1 { self?.reportTaps("done") }
            DispatchQueue.main.asyncAfter(deadline: .now() + (pressed ? 1.5 : 0.5)) {
                self?.tap(pressed ? Array(selectors.dropFirst()) : selectors, attempt: pressed ? 0 : attempt + 1)
            }
        }
    }

    private func reportTaps(_ outcome: String) {
        try? outcome.write(toFile: NSTemporaryDirectory() + "hide-debug-taps", atomically: true, encoding: .utf8)
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
