package me.grab.hide.android;

import android.content.Context;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.util.List;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * The phone's one secret, the twin of the iOS app's {@code PairingKeychain}: for each computer, the credential that
 * computer gave it with the computer's Noise public key, under the computer's room. Nothing else is kept here; the
 * computer's name and the rest of the host list stay in the web view's storage, where no secret is.
 *
 * <p>Each item is a sealed file ({@link SealedFiles}) in the app's no-backup folder under the store's own Keystore key,
 * which can be used only while the phone is unlocked. The closed set of calls below is the whole interface, each argument
 * checked as iOS checks it, at most 16 items.
 */
public final class PairingStore {

    /** The same limit as the host list on the web side ({@code MAX_HOSTS} in hosts.ts). */
    public static final int MAX_ITEMS = 16;
    public static final String KEY_ALIAS = "me.grab.hide.android.pairing";

    public static final class Item {

        public final String room;
        public final String noiseKey;
        /** Null once the computer refused it: the item stays so the host's row can say so, without the credential. */
        public final String credential;

        public Item(String room, String noiseKey, String credential) {
            this.room = room;
            this.noiseKey = noiseKey;
            this.credential = credential;
        }
    }

    private final SealedFiles files;

    public PairingStore(File folder, String alias) {
        files = new SealedFiles(folder, ".pairing", alias, true);
    }

    /** The app's own store: the no-backup folder and the app's key. */
    public static PairingStore of(Context context) {
        return new PairingStore(new File(context.getNoBackupFilesDir(), "pairing"), KEY_ALIAS);
    }

    public static boolean isKey(String value) {
        return SealedFiles.isRoom(value);
    }

    /** The field checks of a save, the same as iOS: a 43-character base64url room and Noise key, a credential of 1 to 512 bytes when given. */
    public static String refusal(Item item) {
        if (!isKey(item.room)) return "invalid_room";
        if (!isKey(item.noiseKey)) return "invalid_field";
        if (item.credential != null && (item.credential.isEmpty() || item.credential.getBytes(StandardCharsets.UTF_8).length > 512)) return "invalid_field";
        return null;
    }

    public synchronized void save(Item item) throws SealedFiles.Failure {
        String refused = refusal(item);
        if (refused != null) throw new SealedFiles.Failure(refused);
        if (!files.has(item.room) && files.rooms().size() >= MAX_ITEMS) throw new SealedFiles.Failure("too_many");
        try {
            JSONObject body = new JSONObject();
            body.put("noiseKey", item.noiseKey);
            if (item.credential != null) body.put("credential", item.credential);
            files.seal(item.room, body.toString().getBytes(StandardCharsets.UTF_8));
        } catch (JSONException e) {
            throw new SealedFiles.Failure("keychain_failed", e);
        }
    }

    public synchronized Item read(String room) throws SealedFiles.Failure {
        if (!isKey(room)) throw new SealedFiles.Failure("invalid_room");
        byte[] opened = files.open(room);
        if (opened == null) return null;
        try {
            JSONObject body = new JSONObject(new String(opened, StandardCharsets.UTF_8));
            String noiseKey = body.optString("noiseKey", null);
            if (!isKey(noiseKey)) throw new SealedFiles.Failure("corrupt");
            return new Item(room, noiseKey, body.has("credential") ? body.getString("credential") : null);
        } catch (JSONException e) {
            throw new SealedFiles.Failure("corrupt", e);
        }
    }

    public synchronized void remove(String room) throws SealedFiles.Failure {
        if (!isKey(room)) throw new SealedFiles.Failure("invalid_room");
        files.remove(room);
    }

    /** The rooms that have an item, never a secret. */
    public synchronized List<String> list() throws SealedFiles.Failure {
        return files.rooms();
    }

    /** Every item and the key: for tests. */
    public synchronized void removeAll() throws SealedFiles.Failure {
        files.removeAll();
    }
}
