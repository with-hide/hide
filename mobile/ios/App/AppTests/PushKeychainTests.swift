import Security
import UIKit
import XCTest
import HidePushCore
@testable import HideShell

/// The push keys against the simulator's real Keychain from inside the app, in the shared group the extension also names.
final class PushKeychainTests: XCTestCase {
    private let room = String(repeating: "r", count: 43)
    private let other = String(repeating: "s", count: 43)
    private var keychain: PushKeychain!
    private var service: String!

    override func setUpWithError() throws {
        service = "me.grab.hide.ios.tests.push.\(UUID().uuidString)"
        keychain = PushKeychain(accessGroup: PushKeychain.sharedAccessGroup(), service: service)
    }

    override func tearDownWithError() throws {
        try keychain.removeAll()
    }

    func testTheSharedGroupIsNamedFromTheBuildAndIsNotThePairingGroup() throws {
        let shared = try XCTUnwrap(PushKeychain.sharedAccessGroup())
        XCTAssertTrue(shared.hasSuffix(".me.grab.hide.ios.push-keys"))
        XCTAssertNotEqual(shared, PairingKeychain.ownAccessGroup())
    }

    func testAKeyIsMadeReadListedReplacedAndRemoved() throws {
        XCTAssertEqual(try keychain.rooms(), [])
        let made = try keychain.create(room: room)
        XCTAssertEqual(made.privateKey.count, 32)
        XCTAssertEqual(made.auth.count, 16)
        XCTAssertEqual(made.publicKey.count, 65)
        XCTAssertEqual(try keychain.read(room: room), made)
        XCTAssertEqual(try keychain.all(), [made])
        let replaced = try keychain.create(room: room)
        XCTAssertNotEqual(replaced.privateKey, made.privateKey, "turning notifications on again makes a new key")
        XCTAssertEqual(try keychain.rooms(), [room])
        try keychain.remove(room: room)
        XCTAssertNil(try keychain.read(room: room))
        XCTAssertEqual(try keychain.all(), [])
    }

    func testARemovedComputersKeyIsTheOnlyOneThatGoes() throws {
        let kept = try keychain.create(room: room)
        try keychain.create(room: other)
        try keychain.remove(room: other)
        XCTAssertEqual(try keychain.all(), [kept])
    }

    func testAtMostSixteenComputersAndAnInvalidRoomIsRefused() throws {
        for index in 0..<PushKeychain.maxItems {
            try keychain.create(room: String(repeating: Character(UnicodeScalar(65 + index)!), count: 43))
        }
        XCTAssertThrowsError(try keychain.create(room: room)) { XCTAssertEqual($0 as? PushKeychainError, .tooMany) }
        XCTAssertThrowsError(try keychain.create(room: "short")) { XCTAssertEqual($0 as? PushKeychainError, .invalidRoom) }
        XCTAssertThrowsError(try keychain.read(room: "../x")) { XCTAssertEqual($0 as? PushKeychainError, .invalidRoom) }
    }

    /// Readable after the first unlock (a notice arrives while the phone is locked), this device only, not synchronized,
    /// and in the shared group alone: the app's own group, where the pairing credential lives, holds none.
    func testTheItemIsThisDeviceOnlyNotSynchronizedAndOutsideThePairingGroup() throws {
        try keychain.create(room: room)
        func attributes(group: String) -> [String: Any]? {
            var result: CFTypeRef?
            let query: [String: Any] = [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service!,
                kSecAttrAccessGroup as String: group,
                kSecAttrSynchronizable as String: kSecAttrSynchronizableAny,
                kSecReturnAttributes as String: true,
            ]
            return SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess ? result as? [String: Any] : nil
        }
        let found = try XCTUnwrap(attributes(group: try XCTUnwrap(PushKeychain.sharedAccessGroup())))
        XCTAssertEqual(found[kSecAttrAccessible as String] as? String, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String)
        XCTAssertEqual(found[kSecAttrSynchronizable as String] as? Bool ?? false, false)
        XCTAssertNil(attributes(group: try XCTUnwrap(PairingKeychain.ownAccessGroup())), "no push key in the pairing group")
    }

    /// A pairing item is not a push key: the extension's query, by the push service, never meets it.
    func testAPairingItemIsNotSeenByThePushKeys() throws {
        let pairing = PairingKeychain(service: "me.grab.hide.ios.tests.pairing.\(UUID().uuidString)", accessGroup: PairingKeychain.ownAccessGroup())
        defer { try? pairing.removeAll() }
        try pairing.save(PairingItem(room: room, noiseKey: String(repeating: "k", count: 43), credential: "secret"))
        XCTAssertEqual(try keychain.all(), [])
    }
}
