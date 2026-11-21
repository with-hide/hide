import Foundation
import Security

/// What the phone keeps secret about one computer: the credential that computer gave it, with the computer's Noise
/// public key, under the computer's room. Nothing else is kept here; the computer's name and the rest of the host list
/// stay in the web view's storage, where no secret is.
public struct PairingItem: Equatable {
    public let room: String
    public let noiseKey: String
    /// Nil once the computer refused it: the item stays so the host's row can say so, without the credential.
    public let credential: String?

    public init(room: String, noiseKey: String, credential: String?) {
        self.room = room
        self.noiseKey = noiseKey
        self.credential = credential
    }
}

public enum PairingKeychainError: Error, Equatable {
    case invalidRoom
    case invalidField
    case tooMany
    case corrupt
    case status(OSStatus)

    /// The word the web side gets; a status code is the system's and goes no further.
    public var code: String {
        switch self {
        case .invalidRoom: return "invalid_room"
        case .invalidField: return "invalid_field"
        case .tooMany: return "too_many"
        case .corrupt: return "corrupt"
        case .status: return "keychain_failed"
        }
    }
}

/// The phone's one secret, in the Keychain, one item per computer, and no other way into the Keychain: the closed
/// set of calls below is the whole interface. An item is only readable while the phone is unlocked, never leaves this
/// device (no backup, no iCloud Keychain) and is found by the room the web side names, never by a key it makes up.
public final class PairingKeychain {
    /// The same limit as the host list on the web side (`MAX_HOSTS` in hosts.ts).
    public static let maxItems = 16
    public static let defaultService = "me.grab.hide.ios.pairing"

    private let service: String
    /// The Keychain group every item lives in; nil only where there is no app bundle to name it (the package's own tests).
    public let accessGroup: String?
    private let lock = NSLock()

    /// `accessGroup` names the Keychain group every item is written to and read from. The app passes its own
    /// (`ownAccessGroup`), so a shared group added to the entitlements later for another extension cannot become the
    /// default group the pairing secret lands in; a test that has no app bundle passes none.
    public init(service: String = PairingKeychain.defaultService, accessGroup: String? = nil) {
        self.service = service
        self.accessGroup = accessGroup
    }

    /// The app's own Keychain group, `<team prefix>.<bundle id>`: the build writes the team prefix into the Info.plist
    /// (`AppIdentifierPrefix`, which ends with a dot), so the name needs no entitlement read and cannot drift from the
    /// signing. Nil when the running bundle carries no prefix, which is the package tests' bundle and nothing else.
    public static func ownAccessGroup(bundle: Bundle = .main) -> String? {
        guard let prefix = bundle.object(forInfoDictionaryKey: "AppIdentifierPrefix") as? String, prefix.hasSuffix("."), prefix.count > 1,
              let id = bundle.bundleIdentifier else { return nil }
        return prefix + id
    }

    private static let key = try! NSRegularExpression(pattern: "^[A-Za-z0-9_-]{43}$")

    static func isKey(_ value: String) -> Bool {
        key.firstMatch(in: value, range: NSRange(value.startIndex..., in: value)) != nil
    }

    private func base(_ room: String?) -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            // Items made before this call existed cannot be synchronized; the query asks for the local ones only.
            kSecAttrSynchronizable as String: kCFBooleanFalse as Any,
        ]
        if let accessGroup { query[kSecAttrAccessGroup as String] = accessGroup }
        if let room { query[kSecAttrAccount as String] = room }
        return query
    }

    public func save(_ item: PairingItem) throws {
        guard Self.isKey(item.room) else { throw PairingKeychainError.invalidRoom }
        guard Self.isKey(item.noiseKey) else { throw PairingKeychainError.invalidField }
        if let credential = item.credential, credential.isEmpty || credential.utf8.count > 512 { throw PairingKeychainError.invalidField }
        var body: [String: String] = ["noiseKey": item.noiseKey]
        if let credential = item.credential { body["credential"] = credential }
        let data = try JSONSerialization.data(withJSONObject: body, options: [.sortedKeys])
        lock.lock(); defer { lock.unlock() }
        let exists = try rooms().contains(item.room)
        if !exists, try rooms().count >= Self.maxItems { throw PairingKeychainError.tooMany }
        let status: OSStatus
        if exists {
            status = SecItemUpdate(base(item.room) as CFDictionary, [
                kSecValueData as String: data,
                kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            ] as CFDictionary)
        } else {
            var add = base(item.room)
            add[kSecValueData as String] = data
            add[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
            status = SecItemAdd(add as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw PairingKeychainError.status(status) }
    }

    public func read(room: String) throws -> PairingItem? {
        guard Self.isKey(room) else { throw PairingKeychainError.invalidRoom }
        lock.lock(); defer { lock.unlock() }
        var query = base(room)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else { throw PairingKeychainError.status(status) }
        guard let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: String], let noiseKey = body["noiseKey"], Self.isKey(noiseKey) else {
            throw PairingKeychainError.corrupt
        }
        return PairingItem(room: room, noiseKey: noiseKey, credential: body["credential"])
    }

    public func remove(room: String) throws {
        guard Self.isKey(room) else { throw PairingKeychainError.invalidRoom }
        lock.lock(); defer { lock.unlock() }
        let status = SecItemDelete(base(room) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw PairingKeychainError.status(status) }
    }

    /// The rooms that have an item, never a secret.
    public func list() throws -> [String] {
        lock.lock(); defer { lock.unlock() }
        return try rooms()
    }

    /// Every item this app made under its service: for the first launch after an install, and for tests.
    public func removeAll() throws {
        lock.lock(); defer { lock.unlock() }
        let status = SecItemDelete(base(nil) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw PairingKeychainError.status(status) }
    }

    private func rooms() throws -> [String] {
        var query = base(nil)
        query[kSecReturnAttributes as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitAll
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess, let rows = result as? [[String: Any]] else { throw PairingKeychainError.status(status) }
        return rows.compactMap { $0[kSecAttrAccount as String] as? String }.sorted()
    }

    /// The Keychain outlives the app: a delete and a new install find the old items. A marker in the app's own
    /// defaults, which a delete does take away, says this install has already cleaned up; without it the items of an
    /// earlier install are removed, so a phone never shows or uses a computer a person removed along with the app.
    public func clearLeftoverFromEarlierInstall(defaults: UserDefaults = .standard, marker: String = "me.grab.hide.ios.first-run.v1") throws {
        guard !defaults.bool(forKey: marker) else { return }
        try removeAll()
        defaults.set(true, forKey: marker)
    }
}
