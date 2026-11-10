import Capacitor
import UIKit
import WebKit

/// Capacitor's view controller with the app's guards put in place once the bridge exists.
@objc(ShellViewController)
public final class ShellViewController: CAPBridgeViewController {
    private var navigationPolicy: NavigationPolicy?

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
}
