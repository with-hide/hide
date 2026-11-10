import Capacitor
import UIKit
import WebKit

/// Capacitor's view controller with the app's guards put in place once the bridge exists.
@objc(ShellViewController)
public final class ShellViewController: CAPBridgeViewController {
    private var navigationPolicy: NavigationPolicy?

    /// The phone site is several pages (the loader and one bundle per protocol), not one page.
    public override func router() -> Router {
        PhoneRouter()
    }

    public override func capacitorDidLoad() {
        guard let handler = (bridge as? CapacitorBridge)?.webViewDelegationHandler, let webView else {
            // Without the handler there is nothing to guard, and an unguarded bridge must not run.
            fatalError("Capacitor did not provide its web view handler")
        }
        BridgeGate.install(on: handler.contentController, inner: handler)
        let policy = NavigationPolicy(inner: handler)
        navigationPolicy = policy
        webView.navigationDelegate = policy
    }

#if DEBUG
    /// The simulator check's input, in a debug build only: the simulator has no camera and nothing outside can
    /// type into the web view, so a pairing link given as the launch value `HIDE_DEBUG_PAIR_LINK` is followed as
    /// the loader follows a link opened on the page, and Connect is pressed once the page shows it. A release
    /// build has no such way in.
    public override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard let link = ProcessInfo.processInfo.environment["HIDE_DEBUG_PAIR_LINK"],
              let fragment = URL(string: link)?.fragment, fragment.hasPrefix("pair="),
              let literal = try? JSONEncoder().encode("#" + fragment), let source = String(data: literal, encoding: .utf8) else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
            self?.webView?.evaluateJavaScript("location.hash = \(source)")
            self?.pressConnect(attempt: 0)
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
