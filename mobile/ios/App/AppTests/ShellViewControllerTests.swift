import Capacitor
import UIKit
import WebKit
import XCTest
import HideShell

/// A plugin that records each call that reaches it, in place of one the app will link.
@objc(ProbePlugin)
public final class ProbePlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "ProbePlugin"
    public let jsName = "Probe"
    public let pluginMethods: [CAPPluginMethod] = [CAPPluginMethod(name: "ping", returnType: CAPPluginReturnPromise)]
    private let lock = NSLock()
    private var ids: [String] = []
    var onPing: ((String) -> Void)?

    var calls: [String] { lock.lock(); defer { lock.unlock() }; return ids }

    @objc func ping(_ call: CAPPluginCall) {
        lock.lock(); ids.append(call.callbackId); lock.unlock()
        onPing?(call.callbackId)
        call.resolve()
    }
}

/// The app's own controller, over a folder of test pages instead of the app's bundle, exposing the probe alone.
final class TestShell: ShellViewController {
    let probe = ProbePlugin()
    var folder: URL!

    override var exposedPlugins: Set<String> { ["Probe"] }

    override func instanceDescriptor() -> InstanceDescriptor {
        let descriptor = super.instanceDescriptor()
        descriptor.appLocation = folder
        return descriptor
    }

    override func capacitorDidLoad() {
        super.capacitorDidLoad()
        bridge?.registerPluginInstance(probe)
    }
}

final class ShellViewControllerTests: XCTestCase {
    private static func call(_ id: String) -> String {
        "webkit.messageHandlers.bridge.postMessage({type:'message',pluginId:'Probe',methodName:'ping',callbackId:'\(id)',options:{}})"
    }

    private var folder: URL!
    private var retained: [AnyObject] = []

    override func setUpWithError() throws {
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("shell-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        // What the phone app does with agent-made HTML: a sandboxed frame of its own scheme; the same-origin frame is
        // the worse case, a frame that is not sandboxed at all.
        let pages = [
            "index.html": """
            <!doctype html><body><script>
            \(Self.call("main"))
            let done = 0;
            addEventListener('message', () => { if (++done === 2) setTimeout(() => { \(Self.call("main-final")) }, 400); });
            </script>
            <iframe sandbox="allow-scripts" src="/sandboxed.html"></iframe>
            <iframe src="/plain.html"></iframe></body>
            """,
            "sandboxed.html": "<!doctype html><script>\(Self.call("sandboxed-frame")); parent.postMessage('done', '*')</script>",
            "plain.html": "<!doctype html><script>\(Self.call("same-origin-frame")); parent.postMessage('done', '*')</script>",
        ]
        for (name, text) in pages { try text.write(to: folder.appendingPathComponent(name), atomically: true, encoding: .utf8) }
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: folder)
        retained = []
    }

    /// What reached a plugin through the app's own controller, once the page ran to its last call.
    func testTheControllerRefusesFramesAndServesTheMainFrame() throws {
        let shell = TestShell()
        shell.folder = folder
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        window.rootViewController = shell
        window.makeKeyAndVisible()
        retained = [shell, window]
        let finished = expectation(description: "page ran to its last call")
        shell.probe.onPing = { if $0 == "main-final" { finished.fulfill() } }
        wait(for: [finished], timeout: 30)
        XCTAssertEqual(shell.probe.calls.sorted(), ["main", "main-final"])
    }
}
