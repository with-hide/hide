import CryptoKit
import Foundation
import HidePushCore

/// The computer's half of a notice, for tests that hand the extension a body: RFC 8291 `aes128gcm` of one notice to a
/// key's public half, laid out as `docs/RELAY_PROTOCOL.md` fixes it. The bytes hided makes are checked against the RFC's
/// vector in hided and in `NotificationCipherTests`; this only has to agree with them.
enum NoticeSealer {
    static func seal(_ notice: [String: Any], to key: PushKey) throws -> String {
        func hkdf(_ secret: Data, salt: Data, info: Data, count: Int) -> Data {
            let derived = HKDF<SHA256>.deriveKey(inputKeyMaterial: SymmetricKey(data: secret), salt: salt, info: info, outputByteCount: count)
            return derived.withUnsafeBytes { Data($0) }
        }
        let plain = try JSONSerialization.data(withJSONObject: notice) + Data([2])
        let sender = P256.KeyAgreement.PrivateKey()
        let peer = try P256.KeyAgreement.PublicKey(x963Representation: key.publicKey)
        let secret = try sender.sharedSecretFromKeyAgreement(with: peer).withUnsafeBytes { Data($0) }
        let senderPublic = sender.publicKey.x963Representation
        let ikm = hkdf(secret, salt: key.auth, info: Data("WebPush: info\0".utf8) + key.publicKey + senderPublic, count: 32)
        let salt = Data((0..<16).map { _ in UInt8.random(in: 0...255) })
        let cek = hkdf(ikm, salt: salt, info: Data("Content-Encoding: aes128gcm\0".utf8), count: 16)
        let nonce = hkdf(ikm, salt: salt, info: Data("Content-Encoding: nonce\0".utf8), count: 12)
        let box = try AES.GCM.seal(plain, using: SymmetricKey(data: cek), nonce: AES.GCM.Nonce(data: nonce))
        var body = salt
        body += Data([0, 0, 0x10, 0, 65])
        body += senderPublic
        body += box.ciphertext + box.tag
        return Base64URL.encode(body)
    }

    static func notice(room: String, kind: String = "question") -> [String: Any] {
        var notice: [String: Any] = [
            "title": "Fix the build", "state": "needs_you", "status_code": kind == "approval" ? "approval" : "question",
            "detail": kind == "approval" ? "Run cargo test in the worktree?" : "Should I update the lockfile too?",
            "place": "hide / feat-ios-push-app", "tag": "dev|w1:p2", "device_id": "dev", "pane_id": "w1:p2", "clear": [String](), "room": room,
        ]
        if kind == "question" || kind == "approval" { notice["ask"] = ["kind": kind, "turn": "1794300000000"] }
        return notice
    }
}
