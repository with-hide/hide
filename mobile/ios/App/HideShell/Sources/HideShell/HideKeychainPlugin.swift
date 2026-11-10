import Capacitor
import Foundation

/// The web app's way to the pairing Keychain: save, read, remove and list a computer's item by its room, and nothing
/// else. There is no call that takes a key of the caller's own, so a script of the page cannot read an item that is
/// not a pairing (and a frame that is not the app's main frame never reaches this class: `BridgeGate`).
@objc(HideKeychainPlugin)
open class HideKeychainPlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "HideKeychainPlugin"
    public let jsName = "HideKeychain"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "save", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "read", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "remove", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "list", returnType: CAPPluginReturnPromise),
    ]

    public let keychain: PairingKeychain

    public override init() {
        keychain = PairingKeychain()
        super.init()
    }

    public init(keychain: PairingKeychain) {
        self.keychain = keychain
        super.init()
    }

    open override func load() {
        // Before any call from the page: the items of an earlier install do not belong to this one.
        do { try keychain.clearLeftoverFromEarlierInstall() }
        catch { CAPLog.print("keychain first-run cleanup failed") }
    }

    private func fail(_ call: CAPPluginCall, _ error: Error) {
        let code = (error as? PairingKeychainError)?.code ?? "keychain_failed"
        call.reject(code, code)
    }

    @objc func save(_ call: CAPPluginCall) {
        do {
            try keychain.save(PairingItem(room: call.getString("room") ?? "", noiseKey: call.getString("noiseKey") ?? "", credential: call.getString("credential")))
            call.resolve()
        } catch { fail(call, error) }
    }

    @objc open func read(_ call: CAPPluginCall) {
        do {
            guard let item = try keychain.read(room: call.getString("room") ?? "") else { call.resolve(["found": false]); return }
            var answer: [String: Any] = ["found": true, "noiseKey": item.noiseKey]
            if let credential = item.credential { answer["credential"] = credential }
            call.resolve(answer)
        } catch { fail(call, error) }
    }

    @objc func remove(_ call: CAPPluginCall) {
        do { try keychain.remove(room: call.getString("room") ?? ""); call.resolve() }
        catch { fail(call, error) }
    }

    @objc func list(_ call: CAPPluginCall) {
        do { call.resolve(["rooms": try keychain.list()]) }
        catch { fail(call, error) }
    }
}
