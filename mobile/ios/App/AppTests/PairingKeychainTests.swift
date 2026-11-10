import Security
import XCTest
@testable import HideShell

/// The Keychain behind the pairing plugin, against the simulator's real Keychain from inside the app.
final class PairingKeychainTests: XCTestCase {
    private let room = String(repeating: "r", count: 43)
    private let key = String(repeating: "k", count: 43)
    private var keychain: PairingKeychain!
    private var service: String!

    override func setUpWithError() throws {
        service = "me.grab.hide.ios.tests.\(UUID().uuidString)"
        keychain = PairingKeychain(service: service)
    }

    override func tearDownWithError() throws {
        try keychain.removeAll()
    }

    func testAnItemIsSavedReadListedAndRemoved() throws {
        XCTAssertEqual(try keychain.list(), [])
        try keychain.save(PairingItem(room: room, noiseKey: key, credential: "secret"))
        XCTAssertEqual(try keychain.read(room: room), PairingItem(room: room, noiseKey: key, credential: "secret"))
        XCTAssertEqual(try keychain.list(), [room])
        try keychain.save(PairingItem(room: room, noiseKey: key, credential: nil))
        XCTAssertNil(try keychain.read(room: room)?.credential)
        try keychain.remove(room: room)
        XCTAssertNil(try keychain.read(room: room))
        XCTAssertEqual(try keychain.list(), [])
    }

    /// The items are readable only while the phone is unlocked, stay on this device and are not synchronized.
    func testTheItemIsThisDeviceOnlyAndNotSynchronized() throws {
        try keychain.save(PairingItem(room: room, noiseKey: key, credential: "secret"))
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service!,
            kSecAttrSynchronizable as String: kSecAttrSynchronizableAny,
            kSecReturnAttributes as String: true,
        ]
        var result: CFTypeRef?
        XCTAssertEqual(SecItemCopyMatching(query as CFDictionary, &result), errSecSuccess)
        let row = try XCTUnwrap(result as? [String: Any])
        XCTAssertEqual(row[kSecAttrAccessible as String] as? String, kSecAttrAccessibleWhenUnlockedThisDeviceOnly as String)
        XCTAssertEqual(row[kSecAttrSynchronizable as String] as? Bool ?? false, false)
    }

    func testOnlyAKeyShapedRoomAndFieldsAreAccepted() {
        XCTAssertThrowsError(try keychain.read(room: "../x")) { XCTAssertEqual($0 as? PairingKeychainError, .invalidRoom) }
        XCTAssertThrowsError(try keychain.remove(room: "")) { XCTAssertEqual($0 as? PairingKeychainError, .invalidRoom) }
        XCTAssertThrowsError(try keychain.save(PairingItem(room: room, noiseKey: "short", credential: nil))) { XCTAssertEqual($0 as? PairingKeychainError, .invalidField) }
        XCTAssertThrowsError(try keychain.save(PairingItem(room: room, noiseKey: key, credential: String(repeating: "x", count: 513)))) { XCTAssertEqual($0 as? PairingKeychainError, .invalidField) }
    }

    func testTheSeventeenthComputerIsRefused() throws {
        for index in 0..<PairingKeychain.maxItems {
            try keychain.save(PairingItem(room: String(format: "%043d", index), noiseKey: key, credential: "c"))
        }
        XCTAssertThrowsError(try keychain.save(PairingItem(room: room, noiseKey: key, credential: "c"))) { XCTAssertEqual($0 as? PairingKeychainError, .tooMany) }
        try keychain.save(PairingItem(room: String(format: "%043d", 3), noiseKey: key, credential: "again"))
    }

    /// A delete and a new install leave the Keychain behind and take the defaults away: the marker's absence says so.
    func testAFreshInstallClearsWhatAnEarlierOneLeft() throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "me.grab.hide.ios.tests.\(UUID().uuidString)"))
        try keychain.save(PairingItem(room: room, noiseKey: key, credential: "old"))
        try keychain.clearLeftoverFromEarlierInstall(defaults: defaults, marker: "first-run")
        XCTAssertEqual(try keychain.list(), [])
        try keychain.save(PairingItem(room: room, noiseKey: key, credential: "new"))
        try keychain.clearLeftoverFromEarlierInstall(defaults: defaults, marker: "first-run")
        XCTAssertEqual(try keychain.read(room: room)?.credential, "new")
    }
}
