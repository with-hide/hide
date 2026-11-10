import CryptoKit
import Foundation
import Security

public struct PushKey: Equatable {
    public let room: String
    /// The P-256 private key, 32 raw bytes: it opens this computer's notices and nothing else.
    public let privateKey: Data
    /// The 16-byte authentication secret of the Web Push subscription shape (RFC 8291).
    public let auth: Data

    /// The 65-byte uncompressed public key hided encrypts to.
    public var publicKey: Data {
        (try? P256.KeyAgreement.PrivateKey(rawRepresentation: privateKey).publicKey.x963Representation) ?? Data()
    }
}

public enum PushKeychainError: Error, Equatable {
    case invalidRoom
    case tooMany
    case corrupt
    case status(OSStatus)

    /// The word the web side gets; a status code is the system's and goes no further.
    public var code: String {
        switch self {
        case .invalidRoom: return "invalid_room"
        case .tooMany: return "too_many"
        case .corrupt: return "corrupt"
        case .status: return "keychain_failed"
        }
    }
}

/// The per-computer push keys, one Keychain item per computer in ONE access group the app and its notification
/// extension both name. The pairing credential is another item in the app's own group, which the extension's
/// entitlements do not list, so the extension can decrypt a notice and cannot connect to a computer.
/// An item is readable after the first unlock since boot (a notice arrives while the phone is locked), never
/// synchronized and never moved to another device.
public final class PushKeychain {
    /// The same limit as the host list on the web side (`MAX_HOSTS` in hosts.ts).
    public static let maxItems = 16
    public static let service = "me.grab.hide.ios.push"
    /// The group's name without the team prefix, as the entitlements spell it after `$(AppIdentifierPrefix)`.
    public static let groupSuffix = "me.grab.hide.ios.push-keys"

    private let accessGroup: String?
    private let itemService: String
    private let lock = NSLock()

    /// `accessGroup` is the shared group; nil only where there is no app bundle to name one (a package's own tests).
    public init(accessGroup: String?, service: String = PushKeychain.service) {
        self.accessGroup = accessGroup
        itemService = service
    }

    /// The shared group, `<team prefix>.me.grab.hide.ios.push-keys`, from the `AppIdentifierPrefix` the build writes into
    /// the Info.plist of the app and of the extension, so neither names it by hand and neither can drift from the signing.
    public static func sharedAccessGroup(bundle: Bundle = .main) -> String? {
        guard let prefix = bundle.object(forInfoDictionaryKey: "AppIdentifierPrefix") as? String, prefix.hasSuffix("."), prefix.count > 1 else { return nil }
        return prefix + groupSuffix
    }

    private static let roomPattern = try! NSRegularExpression(pattern: "^[A-Za-z0-9_-]{43}$")

    public static func isRoom(_ value: String) -> Bool {
        roomPattern.firstMatch(in: value, range: NSRange(value.startIndex..., in: value)) != nil
    }

    private func base(_ room: String?) -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: itemService,
            kSecAttrSynchronizable as String: kCFBooleanFalse as Any,
        ]
        if let accessGroup { query[kSecAttrAccessGroup as String] = accessGroup }
        if let room { query[kSecAttrAccount as String] = room }
        return query
    }

    /// Makes this computer's key pair and secret and stores them, replacing an earlier pair for the room. Only the
    /// public half and the secret ever leave the phone, in the registration the computer asks for.
    @discardableResult
    public func create(room: String) throws -> PushKey {
        guard Self.isRoom(room) else { throw PushKeychainError.invalidRoom }
        lock.lock(); defer { lock.unlock() }
        let exists = try rooms().contains(room)
        if !exists, try rooms().count >= Self.maxItems { throw PushKeychainError.tooMany }
        var auth = Data(count: 16)
        let status = auth.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, 16, $0.baseAddress!) }
        guard status == errSecSuccess else { throw PushKeychainError.status(status) }
        let key = PushKey(room: room, privateKey: P256.KeyAgreement.PrivateKey().rawRepresentation, auth: auth)
        let body = try JSONSerialization.data(withJSONObject: ["k": Base64URL.encode(key.privateKey), "a": Base64URL.encode(key.auth)], options: [.sortedKeys])
        if exists { try remove(room: room, locked: true) }
        var add = base(room)
        add[kSecValueData as String] = body
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let added = SecItemAdd(add as CFDictionary, nil)
        guard added == errSecSuccess else { throw PushKeychainError.status(added) }
        return key
    }

    public func read(room: String) throws -> PushKey? {
        guard Self.isRoom(room) else { throw PushKeychainError.invalidRoom }
        lock.lock(); defer { lock.unlock() }
        var query = base(room)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else { throw PushKeychainError.status(status) }
        return try Self.key(room: room, data: data)
    }

    /// Every key, for the extension to try in turn: Apple is not told which computer a notice is from, so nothing in
    /// the notice names a key.
    public func all() throws -> [PushKey] {
        lock.lock(); defer { lock.unlock() }
        var query = base(nil)
        query[kSecReturnAttributes as String] = true
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitAll
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess, let rows = result as? [[String: Any]] else { throw PushKeychainError.status(status) }
        return try rows.compactMap { row in
            guard let room = row[kSecAttrAccount as String] as? String, Self.isRoom(room), let data = row[kSecValueData as String] as? Data else { return nil }
            return try Self.key(room: room, data: data)
        }
    }

    public func remove(room: String) throws {
        guard Self.isRoom(room) else { throw PushKeychainError.invalidRoom }
        lock.lock(); defer { lock.unlock() }
        try remove(room: room, locked: true)
    }

    private func remove(room: String, locked: Bool) throws {
        let status = SecItemDelete(base(room) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw PushKeychainError.status(status) }
    }

    public func rooms() throws -> [String] {
        var query = base(nil)
        query[kSecReturnAttributes as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitAll
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess, let rows = result as? [[String: Any]] else { throw PushKeychainError.status(status) }
        return rows.compactMap { $0[kSecAttrAccount as String] as? String }.sorted()
    }

    /// Every item of this app's push keys; for the first launch after an install and for tests.
    public func removeAll() throws {
        lock.lock(); defer { lock.unlock() }
        let status = SecItemDelete(base(nil) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw PushKeychainError.status(status) }
    }

    private static func key(room: String, data: Data) throws -> PushKey {
        guard let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: String],
              let privateKey = body["k"].flatMap(Base64URL.decode), privateKey.count == 32,
              let auth = body["a"].flatMap(Base64URL.decode), auth.count == 16,
              (try? P256.KeyAgreement.PrivateKey(rawRepresentation: privateKey)) != nil else { throw PushKeychainError.corrupt }
        return PushKey(room: room, privateKey: privateKey, auth: auth)
    }
}
