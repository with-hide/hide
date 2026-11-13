package me.grab.hide.android;

import java.math.BigInteger;
import java.security.AlgorithmParameters;
import java.security.KeyFactory;
import java.security.SecureRandom;
import java.security.spec.ECGenParameterSpec;
import java.security.spec.ECParameterSpec;
import java.security.spec.ECPoint;
import java.security.spec.ECPrivateKeySpec;
import java.security.spec.ECPublicKeySpec;
import java.util.Arrays;
import javax.crypto.Cipher;
import javax.crypto.KeyAgreement;
import javax.crypto.Mac;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;
import org.json.JSONArray;
import org.json.JSONObject;

/**
 * Encrypts a notice the way hided does (RFC 8291 aes128gcm, one record), for the tests that need a body made for a key
 * they hold: the JVM tests and the instrumented ones share it (src/sharedTest). It is written from the RFC, apart from
 * {@link PushCipher}, so a fault in the cipher is not repeated here.
 */
final class NoticeSealer {

    private NoticeSealer() {}

    static String seal(byte[] plain, byte[] receiverPublic, byte[] auth) throws Exception {
        return seal(plain, receiverPublic, auth, new byte[] { 2 });
    }

    static String seal(byte[] plain, byte[] receiverPublic, byte[] auth, byte[] delimiter) throws Exception {
        PushCipher.Pair sender = PushCipher.generate();
        ECParameterSpec curve = curve();
        KeyAgreement agreement = KeyAgreement.getInstance("ECDH");
        agreement.init(KeyFactory.getInstance("EC").generatePrivate(new ECPrivateKeySpec(new BigInteger(1, sender.privateKey), curve)));
        agreement.doPhase(KeyFactory.getInstance("EC").generatePublic(point(receiverPublic, curve)), true);
        byte[] secret = agreement.generateSecret();
        byte[] salt = new byte[16];
        new SecureRandom().nextBytes(salt);
        byte[] ikm = hmac(hmac(auth, secret), join("WebPush: info\0".getBytes(), receiverPublic, sender.publicKey, new byte[] { 1 }));
        byte[] prk = hmac(salt, ikm);
        byte[] cek = Arrays.copyOf(hmac(prk, join("Content-Encoding: aes128gcm\0".getBytes(), new byte[] { 1 })), 16);
        byte[] nonce = Arrays.copyOf(hmac(prk, join("Content-Encoding: nonce\0".getBytes(), new byte[] { 1 })), 12);
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, new SecretKeySpec(cek, "AES"), new GCMParameterSpec(128, nonce));
        byte[] sealed = cipher.doFinal(join(plain, delimiter));
        return PushCipher.toBase64Url(join(salt, new byte[] { 0, 0, 0x10, 0, 65 }, sender.publicKey, sealed));
    }

    /** A notice as {@code push::payload} builds it, for a room. */
    static JSONObject notice(String room) throws Exception {
        return new JSONObject()
            .put("title", "Fix the build")
            .put("state", "needs_you")
            .put("status_code", "question")
            .put("detail", "Should I update the lockfile too?")
            .put("subject", JSONObject.NULL)
            .put("place", "contong")
            .put("tag", "dev|w1:p2")
            .put("device_id", "dev")
            .put("pane_id", "w1:p2")
            .put("room", room)
            .put("clear", new JSONArray().put("dev|w1:p1"));
    }

    private static ECParameterSpec curve() throws Exception {
        AlgorithmParameters parameters = AlgorithmParameters.getInstance("EC");
        parameters.init(new ECGenParameterSpec("secp256r1"));
        return parameters.getParameterSpec(ECParameterSpec.class);
    }

    private static ECPublicKeySpec point(byte[] uncompressed, ECParameterSpec curve) {
        return new ECPublicKeySpec(new ECPoint(new BigInteger(1, Arrays.copyOfRange(uncompressed, 1, 33)), new BigInteger(1, Arrays.copyOfRange(uncompressed, 33, 65))), curve);
    }

    private static byte[] hmac(byte[] key, byte[] message) throws Exception {
        Mac mac = Mac.getInstance("HmacSHA256");
        mac.init(new SecretKeySpec(key, "HmacSHA256"));
        return mac.doFinal(message);
    }

    private static byte[] join(byte[]... parts) {
        int length = 0;
        for (byte[] part : parts) length += part.length;
        byte[] out = new byte[length];
        int at = 0;
        for (byte[] part : parts) {
            System.arraycopy(part, 0, out, at, part.length);
            at += part.length;
        }
        return out;
    }
}
