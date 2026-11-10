import CryptoKit
import XCTest
@testable import HidePushCore

final class NotificationCipherTests: XCTestCase {
    // RFC 8291 appendix A, the same vector `hided` pins (`the_app_body_vector_is_appendix_a_in_base64url`).
    let vectorBody = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
    let vectorKey = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94"
    let vectorAuth = "BTBZMqHH6r4Tts7J_aSIgg"

    func testOpensTheRfcAppendixAVector() throws {
        let plain = try NotificationCipher.decrypt(Base64URL.decode(vectorBody)!, privateKey: Base64URL.decode(vectorKey)!, auth: Base64URL.decode(vectorAuth)!)
        XCTAssertEqual(String(decoding: plain, as: UTF8.self), "When I grow up, I want to be a watermelon")
    }

    func testAnotherKeyAnAlteredRecordAndAShortOneAreRefusedWithoutOpening() throws {
        let body = Base64URL.decode(vectorBody)!
        let key = Base64URL.decode(vectorKey)!, auth = Base64URL.decode(vectorAuth)!
        let other = P256.KeyAgreement.PrivateKey().rawRepresentation
        XCTAssertThrowsError(try NotificationCipher.decrypt(body, privateKey: other, auth: auth)) { XCTAssertEqual($0 as? NotificationCipherError, .notForThisKey) }
        XCTAssertThrowsError(try NotificationCipher.decrypt(body, privateKey: key, auth: Data(count: 16))) { XCTAssertEqual($0 as? NotificationCipherError, .notForThisKey) }
        var altered = body
        altered[altered.count - 1] ^= 1
        XCTAssertThrowsError(try NotificationCipher.decrypt(altered, privateKey: key, auth: auth)) { XCTAssertEqual($0 as? NotificationCipherError, .notForThisKey) }
        XCTAssertThrowsError(try NotificationCipher.decrypt(body.prefix(60), privateKey: key, auth: auth)) { XCTAssertEqual($0 as? NotificationCipherError, .malformed) }
        var wrongSize = body
        wrongSize[18] = 0x20
        XCTAssertThrowsError(try NotificationCipher.decrypt(wrongSize, privateKey: key, auth: auth)) { XCTAssertEqual($0 as? NotificationCipherError, .malformed) }
        XCTAssertThrowsError(try NotificationCipher.decrypt(body, privateKey: key, auth: Data(count: 15))) { XCTAssertEqual($0 as? NotificationCipherError, .malformed) }
    }

    private func mine() -> PushKey {
        PushKey(room: String(repeating: "k", count: 43), privateKey: P256.KeyAgreement.PrivateKey().rawRepresentation, auth: Data((0..<16).map { UInt8($0) }))
    }

    func testARecordThatOpensButDoesNotEndWithTheLastRecordsDelimiterIsRefused() throws {
        let key = mine()
        let plain = Data("a notice".utf8)
        let good = try encrypt(plain, for: key)
        XCTAssertEqual(try NotificationCipher.decrypt(Base64URL.decode(good)!, privateKey: key.privateKey, auth: key.auth), plain)
        // 0x01 marks a record that is not the last one, no delimiter at all marks nothing, and padding before the
        // delimiter is not zero padding after it.
        for (name, delimiter) in [("not the last record", [UInt8(1)]), ("no delimiter", []), ("padding only", [UInt8(0), 0, 0])] {
            let body = try encrypt(plain, for: key, delimiter: delimiter)
            XCTAssertThrowsError(try NotificationCipher.decrypt(Base64URL.decode(body)!, privateKey: key.privateKey, auth: key.auth), name) {
                XCTAssertEqual($0 as? NotificationCipherError, .malformed, name)
            }
        }
        // Zero padding after the delimiter is part of the record and is dropped.
        let padded = try encrypt(plain, for: key, delimiter: [2, 0, 0, 0])
        XCTAssertEqual(try NotificationCipher.decrypt(Base64URL.decode(padded)!, privateKey: key.privateKey, auth: key.auth), plain)
    }

    func testABodyOverTheLargestTheRelayCarriesIsRefusedBeforeAnyKeyIsTried() throws {
        let key = mine()
        let big = try encrypt(Data(repeating: 0x61, count: 4100), for: key)
        let body = Base64URL.decode(big)!
        XCTAssertGreaterThan(body.count, NotificationCipher.maxBody)
        XCTAssertThrowsError(try NotificationCipher.decrypt(body, privateKey: key.privateKey, auth: key.auth)) {
            XCTAssertEqual($0 as? NotificationCipherError, .malformed)
        }
        // The largest allowed size, to show the limit is the size and not the content.
        let edge = try encrypt(Data(repeating: 0x61, count: NotificationCipher.maxBody - NotificationCipher.headerLength - 17), for: key)
        XCTAssertEqual(Base64URL.decode(edge)!.count, NotificationCipher.maxBody)
        XCTAssertNoThrow(try NotificationCipher.decrypt(Base64URL.decode(edge)!, privateKey: key.privateKey, auth: key.auth))
    }
}

/// Encrypts a notice the way hided does, for the tests that need a body made for a key they hold.
func encrypt(_ plain: Data, for key: PushKey, delimiter: [UInt8] = [2]) throws -> String {
    let sender = P256.KeyAgreement.PrivateKey()
    let receiver = try P256.KeyAgreement.PublicKey(x963Representation: key.publicKey)
    let secret = try sender.sharedSecretFromKeyAgreement(with: receiver).withUnsafeBytes { Data($0) }
    func hmac(_ k: Data, _ m: Data) -> Data { Data(HMAC<SHA256>.authenticationCode(for: m, using: SymmetricKey(data: k))) }
    let salt = Data((0..<16).map { _ in UInt8.random(in: 0...255) })
    let prkKey = hmac(key.auth, secret)
    let ikm = hmac(prkKey, Data("WebPush: info\0".utf8) + key.publicKey + sender.publicKey.x963Representation + Data([1]))
    let prk = hmac(salt, ikm)
    let cek = hmac(prk, Data("Content-Encoding: aes128gcm\0".utf8) + Data([1])).prefix(16)
    let nonce = hmac(prk, Data("Content-Encoding: nonce\0".utf8) + Data([1])).prefix(12)
    let box = try AES.GCM.seal(plain + Data(delimiter), using: SymmetricKey(data: cek), nonce: AES.GCM.Nonce(data: nonce))
    var record = salt + Data([0, 0, 0x10, 0, 65]) + sender.publicKey.x963Representation
    record += box.ciphertext + box.tag
    return Base64URL.encode(record)
}
