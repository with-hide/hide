package me.grab.hide.android;

import android.content.Context;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.List;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * The per-computer notification keys, the twin of the iOS app's {@code PushKeychain}: for each computer, a P-256 key pair
 * and a 16-byte authentication secret (the Web Push subscription's shape, RFC 8291), made on the phone when notifications
 * are turned on for that computer. Only the public half and the secret ever leave the phone, in the registration the
 * computer asks for; the private half opens that computer's notices and nothing else.
 *
 * <p>The keys are sealed files ({@link SealedFiles}) in a folder and under an Android Keystore key of their own, apart
 * from the pairing credentials ({@link PairingStore}), and only this class reads them: the message service decrypts a
 * notice with them and never touches a credential. Unlike the pairing key, theirs can be used while the phone is locked
 * after its first unlock, because a notice arrives on a locked phone; nothing is ever backed up or moved to another device.
 */
public final class PushKeyStore {

    /** The same limit as the host list on the web side ({@code MAX_HOSTS} in hosts.ts). */
    public static final int MAX_ITEMS = 16;
    public static final String KEY_ALIAS = "me.grab.hide.android.push";

    /** One computer's key. */
    public static final class Key {

        public final String room;
        /** The P-256 private scalar, 32 bytes. */
        public final byte[] privateKey;
        /** The 65-byte uncompressed public point hided encrypts to. */
        public final byte[] publicKey;
        public final byte[] auth;

        Key(String room, byte[] privateKey, byte[] publicKey, byte[] auth) {
            this.room = room;
            this.privateKey = privateKey;
            this.publicKey = publicKey;
            this.auth = auth;
        }
    }

    private final SealedFiles files;

    public PushKeyStore(File folder, String alias) {
        files = new SealedFiles(folder, ".push", alias, false);
    }

    /** The app's own store: its no-backup folder and its key. */
    public static PushKeyStore of(Context context) {
        return new PushKeyStore(new File(context.getNoBackupFilesDir(), "push"), KEY_ALIAS);
    }

    /** This computer's key, made and kept now when it has none. */
    public synchronized Key readOrCreate(String room) throws SealedFiles.Failure {
        Key kept = read(room);
        if (kept != null) return kept;
        if (files.rooms().size() >= MAX_ITEMS) throw new SealedFiles.Failure("too_many");
        PushCipher.Pair pair = PushCipher.generate();
        byte[] auth = new byte[16];
        new SecureRandom().nextBytes(auth);
        try {
            JSONObject body = new JSONObject();
            body.put("k", PushCipher.toBase64Url(pair.privateKey));
            body.put("p", PushCipher.toBase64Url(pair.publicKey));
            body.put("a", PushCipher.toBase64Url(auth));
            files.seal(room, body.toString().getBytes(StandardCharsets.UTF_8));
        } catch (JSONException e) {
            throw new SealedFiles.Failure("keychain_failed", e);
        }
        return new Key(room, pair.privateKey, pair.publicKey, auth);
    }

    /** This computer's key, or null when it has none. */
    public synchronized Key read(String room) throws SealedFiles.Failure {
        if (!SealedFiles.isRoom(room)) throw new SealedFiles.Failure("invalid_room");
        byte[] opened = files.open(room);
        return opened == null ? null : key(room, opened);
    }

    /** Every key, for the message service to try in turn: nothing in a notice names its computer to Google. */
    public synchronized List<Key> all() throws SealedFiles.Failure {
        List<Key> keys = new ArrayList<>();
        for (String room : files.rooms()) {
            byte[] opened = files.open(room);
            if (opened != null) keys.add(key(room, opened));
        }
        return keys;
    }

    public synchronized void remove(String room) throws SealedFiles.Failure {
        if (!SealedFiles.isRoom(room)) throw new SealedFiles.Failure("invalid_room");
        files.remove(room);
    }

    public synchronized List<String> rooms() throws SealedFiles.Failure {
        return files.rooms();
    }

    /** Every key and the store's Keystore key: for tests. */
    public synchronized void removeAll() throws SealedFiles.Failure {
        files.removeAll();
    }

    private static Key key(String room, byte[] opened) throws SealedFiles.Failure {
        try {
            JSONObject body = new JSONObject(new String(opened, StandardCharsets.UTF_8));
            byte[] privateKey = PushCipher.fromBase64Url(body.optString("k", ""));
            byte[] publicKey = PushCipher.fromBase64Url(body.optString("p", ""));
            byte[] auth = PushCipher.fromBase64Url(body.optString("a", ""));
            if (privateKey == null || publicKey == null || auth == null || auth.length != 16 || !PushCipher.usable(privateKey, publicKey)) {
                throw new SealedFiles.Failure("corrupt");
            }
            return new Key(room, privateKey, publicKey, auth);
        } catch (JSONException e) {
            throw new SealedFiles.Failure("corrupt", e);
        }
    }
}
