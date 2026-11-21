import CryptoKit
import Foundation

public enum NotificationCipherError: Error, Equatable {
    /// The body is not one aes128gcm record of the shape `docs/RELAY_PROTOCOL.md` fixes.
    case malformed
    /// The record did not open with this key: it was made for another key, or was altered.
    case notForThisKey
}

/// RFC 8291 `aes128gcm` decryption of one Web Push record, as hided encrypts a notice for the key the app registered
/// (`docs/RELAY_PROTOCOL.md`, The notification body): a 16-byte salt, a 4-byte record size of 4096, a 1-byte key
/// length of 65, the sender's 65-byte P-256 key, then the AES-GCM ciphertext of the notice followed by `0x02`.
/// Pure and without I/O, so the notification extension can run it with no network.
public enum NotificationCipher {
    static let recordSize: UInt32 = 4096
    static let headerLength = 16 + 4 + 1 + 65
    /// The largest body the relay carries (3,000 bytes of ciphertext) with room to spare; a larger one is not ours.
    static let maxBody = 4096

    public static func decrypt(_ body: Data, privateKey: Data, auth: Data) throws -> Data {
        let body = Data(body)
        guard body.count > headerLength + 16, body.count <= maxBody else { throw NotificationCipherError.malformed }
        let salt = body.subdata(in: 0..<16)
        let size = body.subdata(in: 16..<20).reduce(UInt32(0)) { ($0 << 8) | UInt32($1) }
        guard size == recordSize, body[20] == 65, auth.count == 16 else { throw NotificationCipherError.malformed }
        let sender = body.subdata(in: 21..<headerLength)
        let sealed = body.subdata(in: headerLength..<body.count)
        guard sealed.count <= Int(size) else { throw NotificationCipherError.malformed }

        let own: P256.KeyAgreement.PrivateKey
        let peer: P256.KeyAgreement.PublicKey
        do {
            own = try P256.KeyAgreement.PrivateKey(rawRepresentation: privateKey)
            peer = try P256.KeyAgreement.PublicKey(x963Representation: sender)
        } catch { throw NotificationCipherError.malformed }
        guard let shared = try? own.sharedSecretFromKeyAgreement(with: peer) else { throw NotificationCipherError.malformed }
        let secret = shared.withUnsafeBytes { Data($0) }

        func hmac(_ key: Data, _ message: Data) -> Data {
            Data(HMAC<SHA256>.authenticationCode(for: message, using: SymmetricKey(data: key)))
        }
        let prkKey = hmac(auth, secret)
        let info = Data("WebPush: info\0".utf8) + own.publicKey.x963Representation + sender + Data([1])
        let prk = hmac(salt, hmac(prkKey, info))
        let key = hmac(prk, Data("Content-Encoding: aes128gcm\0".utf8) + Data([1])).prefix(16)
        let nonce = hmac(prk, Data("Content-Encoding: nonce\0".utf8) + Data([1])).prefix(12)

        let plain: Data
        do {
            let box = try AES.GCM.SealedBox(
                nonce: AES.GCM.Nonce(data: nonce),
                ciphertext: sealed.prefix(sealed.count - 16),
                tag: sealed.suffix(16)
            )
            plain = try AES.GCM.open(box, using: SymmetricKey(data: key))
        } catch { throw NotificationCipherError.notForThisKey }

        // The record ends with its delimiter: 0x02 for the last (here the only) record, then any zero padding.
        var end = plain.count
        while end > 0, plain[end - 1] == 0 { end -= 1 }
        guard end > 0, plain[end - 1] == 2 else { throw NotificationCipherError.malformed }
        return plain.prefix(end - 1)
    }
}
