package me.grab.hide.android;

import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.security.AlgorithmParameters;
import java.security.GeneralSecurityException;
import java.security.KeyFactory;
import java.security.KeyPair;
import java.security.KeyPairGenerator;
import java.security.SecureRandom;
import java.security.interfaces.ECPrivateKey;
import java.security.interfaces.ECPublicKey;
import java.security.spec.ECFieldFp;
import java.security.spec.ECGenParameterSpec;
import java.security.spec.ECParameterSpec;
import java.security.spec.ECPoint;
import java.security.spec.ECPrivateKeySpec;
import java.security.spec.ECPublicKeySpec;
import java.util.Arrays;
import java.util.Base64;
import javax.crypto.AEADBadTagException;
import javax.crypto.Cipher;
import javax.crypto.KeyAgreement;
import javax.crypto.Mac;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;

/**
 * RFC 8291 {@code aes128gcm} decryption of one Web Push record, the twin of the iOS app's {@code NotificationCipher}, as
 * hided encrypts a notice for the key the app registered ({@code docs/RELAY_PROTOCOL.md}, The notification body): a
 * 16-byte salt, a 4-byte record size of 4096, a 1-byte key length of 65, the sender's 65-byte P-256 key, then the AES-GCM
 * ciphertext of the notice followed by {@code 0x02}. Pure and without I/O or Android classes, so a JVM test runs it.
 *
 * <p>A key is kept raw, as on iOS: the 32-byte private scalar and the 65-byte uncompressed public point. Java has no way to
 * derive the public point from the private scalar without its own curve arithmetic, so both are kept.
 */
public final class PushCipher {

    static final int RECORD_SIZE = 4096;
    static final int HEADER_LENGTH = 16 + 4 + 1 + 65;
    /** The largest body the relay carries (3,000 bytes of ciphertext) with room to spare; a larger one is not ours. */
    static final int MAX_BODY = 4096;

    /** Why a body did not open: not one record of the documented shape, or not made for this key (or altered). */
    public static final class Refused extends Exception {

        public final boolean malformed;

        Refused(boolean malformed, Throwable cause) {
            super(malformed ? "malformed" : "not_for_this_key", cause);
            this.malformed = malformed;
        }
    }

    /** One P-256 key pair as the app keeps it. */
    public static final class Pair {

        public final byte[] privateKey;
        public final byte[] publicKey;

        Pair(byte[] privateKey, byte[] publicKey) {
            this.privateKey = privateKey;
            this.publicKey = publicKey;
        }
    }

    private PushCipher() {}

    public static byte[] decrypt(byte[] body, byte[] privateKey, byte[] publicKey, byte[] auth) throws Refused {
        if (body.length <= HEADER_LENGTH + 16 || body.length > MAX_BODY) throw malformed(null);
        if (auth.length != 16 || privateKey.length != 32 || publicKey.length != 65) throw malformed(null);
        int size = ((body[16] & 0xff) << 24) | ((body[17] & 0xff) << 16) | ((body[18] & 0xff) << 8) | (body[19] & 0xff);
        if (size != RECORD_SIZE || body[20] != 65) throw malformed(null);
        byte[] salt = Arrays.copyOfRange(body, 0, 16);
        byte[] sender = Arrays.copyOfRange(body, 21, HEADER_LENGTH);
        byte[] sealed = Arrays.copyOfRange(body, HEADER_LENGTH, body.length);
        if (sealed.length > size) throw malformed(null);

        byte[] secret;
        try {
            KeyAgreement agreement = KeyAgreement.getInstance("ECDH");
            agreement.init(privateKey(privateKey));
            agreement.doPhase(publicKey(sender), true);
            secret = agreement.generateSecret();
        } catch (GeneralSecurityException | IllegalArgumentException e) {
            throw malformed(e);
        }

        byte[] plain;
        try {
            byte[] prkKey = hmac(auth, secret);
            byte[] info = concat("WebPush: info\0".getBytes(StandardCharsets.US_ASCII), publicKey, sender, new byte[] { 1 });
            byte[] prk = hmac(salt, hmac(prkKey, info));
            byte[] key = Arrays.copyOf(hmac(prk, concat("Content-Encoding: aes128gcm\0".getBytes(StandardCharsets.US_ASCII), new byte[] { 1 })), 16);
            byte[] nonce = Arrays.copyOf(hmac(prk, concat("Content-Encoding: nonce\0".getBytes(StandardCharsets.US_ASCII), new byte[] { 1 })), 12);
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.DECRYPT_MODE, new SecretKeySpec(key, "AES"), new GCMParameterSpec(128, nonce));
            plain = cipher.doFinal(sealed);
        } catch (AEADBadTagException e) {
            throw new Refused(false, e);
        } catch (GeneralSecurityException e) {
            throw malformed(e);
        }

        // The record ends with its delimiter: 0x02 for the last (here the only) record, then any zero padding.
        int end = plain.length;
        while (end > 0 && plain[end - 1] == 0) end--;
        if (end == 0 || plain[end - 1] != 2) throw malformed(null);
        return Arrays.copyOf(plain, end - 1);
    }

    /** A new key pair for one computer, from the system's secure random source. */
    public static Pair generate() {
        try {
            KeyPairGenerator generator = KeyPairGenerator.getInstance("EC");
            generator.initialize(new ECGenParameterSpec("secp256r1"), new SecureRandom());
            KeyPair pair = generator.generateKeyPair();
            byte[] scalar = fixed(((ECPrivateKey) pair.getPrivate()).getS(), 32);
            ECPoint point = ((ECPublicKey) pair.getPublic()).getW();
            byte[] uncompressed = concat(new byte[] { 4 }, fixed(point.getAffineX(), 32), fixed(point.getAffineY(), 32));
            return new Pair(scalar, uncompressed);
        } catch (GeneralSecurityException e) {
            throw new IllegalStateException("this system has no P-256 key generator", e);
        }
    }

    /** Whether the two halves are a P-256 key the cipher can use: the scalar in range, the point on the curve. */
    public static boolean usable(byte[] privateKey, byte[] publicKey) {
        try {
            if (privateKey.length != 32) return false;
            privateKey(privateKey);
            publicKey(publicKey);
            return true;
        } catch (GeneralSecurityException | IllegalArgumentException e) {
            return false;
        }
    }

    /** Unpadded base64url (RFC 4648 section 5), the encoding of every key and ciphertext of the push contract; null when it is not. */
    public static byte[] fromBase64Url(String text) {
        if (text == null || !text.matches("[A-Za-z0-9_-]*")) return null;
        try {
            return Base64.getUrlDecoder().decode(text);
        } catch (IllegalArgumentException e) {
            return null;
        }
    }

    public static String toBase64Url(byte[] bytes) {
        return Base64.getUrlEncoder().withoutPadding().encodeToString(bytes);
    }

    private static Refused malformed(Throwable cause) {
        return new Refused(true, cause);
    }

    private static ECParameterSpec curve() throws GeneralSecurityException {
        AlgorithmParameters parameters = AlgorithmParameters.getInstance("EC");
        parameters.init(new ECGenParameterSpec("secp256r1"));
        return parameters.getParameterSpec(ECParameterSpec.class);
    }

    private static java.security.PrivateKey privateKey(byte[] scalar) throws GeneralSecurityException {
        ECParameterSpec curve = curve();
        BigInteger s = new BigInteger(1, scalar);
        if (s.signum() == 0 || s.compareTo(curve.getOrder()) >= 0) throw new GeneralSecurityException("scalar out of range");
        return KeyFactory.getInstance("EC").generatePrivate(new ECPrivateKeySpec(s, curve));
    }

    /**
     * A 65-byte uncompressed point on P-256. Not every key factory checks that the point lies on the curve (the JDK's does
     * not), and an agreement with a point off it can leak the private key, so the curve's equation is checked here.
     */
    private static java.security.PublicKey publicKey(byte[] point) throws GeneralSecurityException {
        if (point.length != 65 || point[0] != 4) throw new GeneralSecurityException("not an uncompressed P-256 point");
        BigInteger x = new BigInteger(1, Arrays.copyOfRange(point, 1, 33));
        BigInteger y = new BigInteger(1, Arrays.copyOfRange(point, 33, 65));
        ECParameterSpec curve = curve();
        BigInteger p = ((ECFieldFp) curve.getCurve().getField()).getP();
        BigInteger right = x.pow(3).add(curve.getCurve().getA().multiply(x)).add(curve.getCurve().getB()).mod(p);
        if (x.compareTo(p) >= 0 || y.compareTo(p) >= 0 || !y.pow(2).mod(p).equals(right)) throw new GeneralSecurityException("point not on P-256");
        return KeyFactory.getInstance("EC").generatePublic(new ECPublicKeySpec(new ECPoint(x, y), curve));
    }

    private static byte[] hmac(byte[] key, byte[] message) throws GeneralSecurityException {
        Mac mac = Mac.getInstance("HmacSHA256");
        mac.init(new SecretKeySpec(key, "HmacSHA256"));
        return mac.doFinal(message);
    }

    /** A non-negative integer as exactly {@code length} big-endian bytes. */
    private static byte[] fixed(BigInteger value, int length) {
        byte[] raw = value.toByteArray();
        if (raw.length == length) return raw;
        byte[] out = new byte[length];
        if (raw.length > length) System.arraycopy(raw, raw.length - length, out, 0, length);
        else System.arraycopy(raw, 0, out, length - raw.length, raw.length);
        return out;
    }

    static byte[] concat(byte[]... parts) {
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
