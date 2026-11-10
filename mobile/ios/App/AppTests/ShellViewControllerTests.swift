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
    override func appPlugins() -> [CAPPlugin] { [probe] }

    override func instanceDescriptor() -> InstanceDescriptor {
        let descriptor = super.instanceDescriptor()
        descriptor.appLocation = folder
        return descriptor
    }

}

/// The app's real Keychain plugin over a service of its own, recording each call that reaches it.
@objc(RecordingKeychain)
final class RecordingKeychain: HideKeychainPlugin {
    private let lock = NSLock()
    private var ids: [String] = []
    var onRead: ((String) -> Void)?
    var calls: [String] { lock.lock(); defer { lock.unlock() }; return ids }

    @objc override func read(_ call: CAPPluginCall) {
        lock.lock(); ids.append(call.callbackId); lock.unlock()
        onRead?(call.callbackId)
        super.read(call)
    }
}

/// The app's own controller with its real exposed list (`BridgeGate.appPlugins`) and the real Keychain plugin.
final class KeychainShell: ShellViewController {
    let keychain = RecordingKeychain(keychain: PairingKeychain(service: "me.grab.hide.ios.tests.\(UUID().uuidString)"))
    var folder: URL!

    override func appPlugins() -> [CAPPlugin] { [keychain] }

    override func instanceDescriptor() -> InstanceDescriptor {
        let descriptor = super.instanceDescriptor()
        descriptor.appLocation = folder
        return descriptor
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

    /// The phone's secret is only for the app's own main frame: a sandboxed frame, which is what agent-made HTML runs
    /// in, and a frame of the app's own origin both ask the Keychain plugin for an item and are never heard; the main
    /// frame is. Take the gate out of `ShellViewController` and the frames' calls arrive, and this fails.
    func testTheKeychainPluginIsReachableFromTheMainFrameOnly() throws {
        func read(_ id: String) -> String {
            "webkit.messageHandlers.bridge.postMessage({type:'message',pluginId:'HideKeychain',methodName:'read',callbackId:'\(id)',options:{room:'\(String(repeating: "a", count: 43))'}})"
        }
        let pages = [
            "index.html": """
            <!doctype html><body><script>
            \(read("main"))
            let done = 0;
            addEventListener('message', () => { if (++done === 2) setTimeout(() => { \(read("main-final")) }, 400); });
            </script>
            <iframe sandbox="allow-scripts" src="/sandboxed.html"></iframe>
            <iframe src="/plain.html"></iframe></body>
            """,
            "sandboxed.html": "<!doctype html><script>\(read("sandboxed-frame")); parent.postMessage('done', '*')</script>",
            "plain.html": "<!doctype html><script>\(read("same-origin-frame")); parent.postMessage('done', '*')</script>",
        ]
        for (name, text) in pages { try text.write(to: folder.appendingPathComponent(name), atomically: true, encoding: .utf8) }
        let shell = KeychainShell()
        shell.folder = folder
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        window.rootViewController = shell
        window.makeKeyAndVisible()
        retained = [shell, window]
        let finished = expectation(description: "page ran to its last call")
        shell.keychain.onRead = { if $0 == "main-final" { finished.fulfill() } }
        wait(for: [finished], timeout: 30)
        XCTAssertEqual(shell.keychain.calls.sorted(), ["main", "main-final"])
        XCTAssertTrue(BridgeGate.appPlugins.isSuperset(of: ["HideKeychain", "HideScanner", "HideApp"]))
    }
}
