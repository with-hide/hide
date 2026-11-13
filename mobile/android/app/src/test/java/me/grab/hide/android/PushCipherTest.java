package me.grab.hide.android;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import org.junit.Test;

/** The RFC 8291 decryption the message service runs, against the RFC's own vector and against bodies the RFC's way makes. */
public class PushCipherTest {

    // RFC 8291 Appendix A, the vector hided pins (`the_app_body_vector_is_appendix_a_in_base64url`) and the iOS app opens.
    static final String VECTOR_BODY = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";
    static final String VECTOR_PRIVATE = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";
    static final String VECTOR_PUBLIC = "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    static final String VECTOR_AUTH = "BTBZMqHH6r4Tts7J_aSIgg";

    private static byte[] b64(String text) {
        return PushCipher.fromBase64Url(text);
    }

    private static boolean malformed(byte[] body, byte[] privateKey, byte[] publicKey, byte[] auth) {
        try {
            PushCipher.decrypt(body, privateKey, publicKey, auth);
        } catch (PushCipher.Refused refused) {
            return refused.malformed;
        }
        throw new AssertionError("the body opened");
    }

    @Test
    public void opensTheRfcAppendixAVector() throws Exception {
        byte[] plain = PushCipher.decrypt(b64(VECTOR_BODY), b64(VECTOR_PRIVATE), b64(VECTOR_PUBLIC), b64(VECTOR_AUTH));
        assertEquals("When I grow up, I want to be a watermelon", new String(plain, StandardCharsets.UTF_8));
    }

    @Test
    public void refusesAnotherKeyAnAlteredRecordAndAShortOneWithoutOpening() {
        byte[] body = b64(VECTOR_BODY), key = b64(VECTOR_PRIVATE), pub = b64(VECTOR_PUBLIC), auth = b64(VECTOR_AUTH);
        PushCipher.Pair other = PushCipher.generate();
        assertFalse(malformed(body, other.privateKey, other.publicKey, auth));
        assertFalse(malformed(body, key, pub, new byte[16]));
        byte[] altered = body.clone();
        altered[altered.length - 1] ^= 1;
        assertFalse(malformed(altered, key, pub, auth));
        assertTrue(malformed(Arrays.copyOf(body, 60), key, pub, auth));
        byte[] wrongSize = body.clone();
        wrongSize[18] = 0x20;
        assertTrue(malformed(wrongSize, key, pub, auth));
        assertTrue(malformed(body, key, pub, new byte[15]));
        byte[] notAPoint = body.clone();
        notAPoint[21] = 3;
        assertTrue(malformed(notAPoint, key, pub, auth));
        // A sender's point off the curve is refused before any agreement is made with it.
        byte[] offCurve = body.clone();
        offCurve[PushCipher.HEADER_LENGTH - 1] ^= 1;
        assertTrue(malformed(offCurve, key, pub, auth));
    }

    @Test
    public void refusesARecordThatOpensButDoesNotEndWithTheLastRecordsDelimiter() throws Exception {
        PushCipher.Pair mine = PushCipher.generate();
        byte[] auth = new byte[16];
        Arrays.fill(auth, (byte) 7);
        byte[] plain = "a notice".getBytes(StandardCharsets.UTF_8);
        assertArrayEquals(plain, PushCipher.decrypt(b64(NoticeSealer.seal(plain, mine.publicKey, auth)), mine.privateKey, mine.publicKey, auth));
        // 0x01 marks a record that is not the last one, no delimiter at all marks nothing, and padding before the
        // delimiter is not zero padding after it.
        for (byte[] delimiter : new byte[][] { { 1 }, {}, { 0, 0, 0 } }) {
            assertTrue(Arrays.toString(delimiter), malformed(b64(NoticeSealer.seal(plain, mine.publicKey, auth, delimiter)), mine.privateKey, mine.publicKey, auth));
        }
        // Zero padding after the delimiter is part of the record and is dropped.
        assertArrayEquals(plain, PushCipher.decrypt(b64(NoticeSealer.seal(plain, mine.publicKey, auth, new byte[] { 2, 0, 0, 0 })), mine.privateKey, mine.publicKey, auth));
    }

    @Test
    public void refusesABodyOverTheLargestTheRelayCarriesBeforeAnyKeyIsTried() throws Exception {
        PushCipher.Pair mine = PushCipher.generate();
        byte[] auth = new byte[16];
        byte[] big = b64(NoticeSealer.seal(new byte[4100], mine.publicKey, auth));
        assertTrue(big.length > PushCipher.MAX_BODY);
        assertTrue(malformed(big, mine.privateKey, mine.publicKey, auth));
        // The largest allowed size, to show the limit is the size and not the content.
        byte[] edge = b64(NoticeSealer.seal(new byte[PushCipher.MAX_BODY - PushCipher.HEADER_LENGTH - 17], mine.publicKey, auth));
        assertEquals(PushCipher.MAX_BODY, edge.length);
        PushCipher.decrypt(edge, mine.privateKey, mine.publicKey, auth);
    }

    @Test
    public void makesKeysThatAreUsableAndReadsOnlyUnpaddedBase64Url() {
        PushCipher.Pair pair = PushCipher.generate();
        assertEquals(32, pair.privateKey.length);
        assertEquals(65, pair.publicKey.length);
        assertEquals(4, pair.publicKey[0]);
        assertTrue(PushCipher.usable(pair.privateKey, pair.publicKey));
        byte[] offCurve = pair.publicKey.clone();
        offCurve[64] ^= 1;
        assertFalse(PushCipher.usable(pair.privateKey, offCurve));
        assertFalse(PushCipher.usable(new byte[32], pair.publicKey));
        assertEquals(VECTOR_AUTH, PushCipher.toBase64Url(b64(VECTOR_AUTH)));
        for (String bad : new String[] { "a+b/", "abc=", "not base64 !", "a" }) {
            if (PushCipher.fromBase64Url(bad) != null) fail(bad);
        }
    }
}
