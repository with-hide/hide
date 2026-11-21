package me.grab.hide.android;

import java.util.List;

/**
 * What the message service does with the ciphertext, apart from Android so a JVM test can state every case: the twin of
 * the iOS app's {@code NoticeOpener}. A notice opens only when one stored key opens the body and the notice names that
 * key's computer; every other case opens nothing.
 */
public final class NoticeOpener {

    private NoticeOpener() {}

    /** {@code hide} is the base64url ciphertext of the FCM data message; Google is not told which computer sent it, so each key is tried. */
    public static Notice open(String hide, List<PushKeyStore.Key> keys) {
        byte[] body = PushCipher.fromBase64Url(hide);
        if (body == null) return null;
        for (PushKeyStore.Key key : keys) {
            byte[] plain;
            try {
                plain = PushCipher.decrypt(body, key.privateKey, key.publicKey, key.auth);
            } catch (PushCipher.Refused refused) {
                continue;
            }
            // The key that opened it belongs to one computer; a notice that names another room is not that computer's.
            Notice notice = Notice.fromJson(plain);
            return notice != null && notice.room.equals(key.room) ? notice : null;
        }
        return null;
    }
}
