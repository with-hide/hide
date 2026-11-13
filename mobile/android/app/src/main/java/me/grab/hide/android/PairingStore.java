package me.grab.hide.android;

import android.content.Context;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.security.GeneralSecurityException;
import java.security.KeyStore;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.regex.Pattern;
import javax.crypto.AEADBadTagException;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * The phone's one secret, the twin of the iOS app's {@code PairingKeychain}: for each computer, the credential that
 * computer gave it with the computer's Noise public key, under the computer's room. Nothing else is kept here; the
 * computer's name and the rest of the host list stay in the web view's storage, where no secret is.
 *
 * <p>Android has no keychain to hold a small secret, so each item is a file in the app's no-backup folder, encrypted with
 * AES-GCM under one Android Keystore key that never leaves the Keystore, can be used only while the phone is unlocked and
 * is not backed up. The room is the cipher's associated data, so a file moved to another room's name does not open. The
 * closed set of calls below is the whole interface, each argument checked as iOS checks it, at most 16 items. Unlike the
 * Keychain, the folder and the key go with the app when it is deleted, so nothing of an earlier install is left to clear.
 */
public final class PairingStore {

    /** The same limit as the host list on the web side ({@code MAX_HOSTS} in hosts.ts). */
    public static final int MAX_ITEMS = 16;
    public static final String KEY_ALIAS = "me.grab.hide.android.pairing";
    private static final Pattern KEY = Pattern.compile("^[A-Za-z0-9_-]{43}$");
    private static final String SUFFIX = ".pairing";
    private static final byte VERSION = 1;
    private static final int IV_BYTES = 12;
    private static final int TAG_BITS = 128;

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

    /** A refusal with the word the web side gets; a system failure's detail goes no further. */
    public static final class Failure extends Exception {

        public final String code;

        Failure(String code) {
            super(code);
            this.code = code;
        }

        Failure(String code, Throwable cause) {
            super(code, cause);
            this.code = code;
        }
    }

    private final File folder;
    private final String alias;

    public PairingStore(File folder, String alias) {
        this.folder = folder;
        this.alias = alias;
    }

    /** The app's own store: the no-backup folder and the app's key. */
    public static PairingStore of(Context context) {
        return new PairingStore(new File(context.getNoBackupFilesDir(), "pairing"), KEY_ALIAS);
    }

    public static boolean isKey(String value) {
        return value != null && KEY.matcher(value).matches();
    }

    /** The field checks of a save, the same as iOS: a 43-character base64url room and Noise key, a credential of 1 to 512 bytes when given. */
    public static String refusal(Item item) {
        if (!isKey(item.room)) return "invalid_room";
        if (!isKey(item.noiseKey)) return "invalid_field";
        if (item.credential != null && (item.credential.isEmpty() || item.credential.getBytes(StandardCharsets.UTF_8).length > 512)) return "invalid_field";
        return null;
    }

    public synchronized void save(Item item) throws Failure {
        String refused = refusal(item);
        if (refused != null) throw new Failure(refused);
        File target = file(item.room);
        if (!target.exists() && rooms().size() >= MAX_ITEMS) throw new Failure("too_many");
        try {
            JSONObject body = new JSONObject();
            body.put("noiseKey", item.noiseKey);
            if (item.credential != null) body.put("credential", item.credential);
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.ENCRYPT_MODE, key(true));
            cipher.updateAAD(item.room.getBytes(StandardCharsets.UTF_8));
            byte[] sealed = cipher.doFinal(body.toString().getBytes(StandardCharsets.UTF_8));
            byte[] iv = cipher.getIV();
            if (iv.length != IV_BYTES) throw new GeneralSecurityException("unexpected iv length");
            byte[] bytes = new byte[1 + IV_BYTES + sealed.length];
            bytes[0] = VERSION;
            System.arraycopy(iv, 0, bytes, 1, IV_BYTES);
            System.arraycopy(sealed, 0, bytes, 1 + IV_BYTES, sealed.length);
            write(target, bytes);
        } catch (GeneralSecurityException | JSONException | IOException e) {
            throw new Failure("keychain_failed", e);
        }
    }

    public synchronized Item read(String room) throws Failure {
        if (!isKey(room)) throw new Failure("invalid_room");
        File source = file(room);
        if (!source.exists()) return null;
        byte[] bytes;
        try {
            bytes = Files.readAllBytes(source.toPath());
        } catch (IOException e) {
            throw new Failure("keychain_failed", e);
        }
        if (bytes.length <= 1 + IV_BYTES || bytes[0] != VERSION) throw new Failure("corrupt");
        try {
            SecretKey key = key(false);
            if (key == null) throw new Failure("corrupt");
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.DECRYPT_MODE, key, new GCMParameterSpec(TAG_BITS, bytes, 1, IV_BYTES));
            cipher.updateAAD(room.getBytes(StandardCharsets.UTF_8));
            byte[] opened = cipher.doFinal(Arrays.copyOfRange(bytes, 1 + IV_BYTES, bytes.length));
            JSONObject body = new JSONObject(new String(opened, StandardCharsets.UTF_8));
            String noiseKey = body.optString("noiseKey", null);
            if (!isKey(noiseKey)) throw new Failure("corrupt");
            return new Item(room, noiseKey, body.has("credential") ? body.getString("credential") : null);
        } catch (AEADBadTagException | JSONException e) {
            throw new Failure("corrupt", e);
        } catch (GeneralSecurityException | IOException e) {
            throw new Failure("keychain_failed", e);
        }
    }

    public synchronized void remove(String room) throws Failure {
        if (!isKey(room)) throw new Failure("invalid_room");
        File target = file(room);
        if (target.exists() && !target.delete()) throw new Failure("keychain_failed");
    }

    /** The rooms that have an item, never a secret. */
    public synchronized List<String> list() {
        return rooms();
    }

    /** Every item and the key: for tests. */
    public synchronized void removeAll() throws Failure {
        for (String room : rooms()) remove(room);
        try {
            KeyStore store = keyStore();
            if (store.containsAlias(alias)) store.deleteEntry(alias);
        } catch (GeneralSecurityException | IOException e) {
            throw new Failure("keychain_failed", e);
        }
    }

    private List<String> rooms() {
        List<String> rooms = new ArrayList<>();
        String[] names = folder.list();
        if (names == null) return rooms;
        for (String name : names) {
            if (!name.endsWith(SUFFIX)) continue;
            String room = name.substring(0, name.length() - SUFFIX.length());
            if (isKey(room)) rooms.add(room);
        }
        Collections.sort(rooms);
        return rooms;
    }

    private File file(String room) {
        return new File(folder, room + SUFFIX);
    }

    /** Written beside the target and moved over it, so a failed write never leaves half an item. */
    private void write(File target, byte[] bytes) throws IOException {
        if (!folder.isDirectory() && !folder.mkdirs()) throw new IOException("pairing folder not made");
        File staged = new File(folder, target.getName() + ".tmp");
        try (FileOutputStream out = new FileOutputStream(staged)) {
            out.write(bytes);
            out.getFD().sync();
        }
        if (!staged.renameTo(target)) {
            staged.delete();
            throw new IOException("pairing item not moved into place");
        }
    }

    private static KeyStore keyStore() throws GeneralSecurityException, IOException {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        return store;
    }

    /** The app's key; made on the first save, null on a read before any save. */
    private SecretKey key(boolean create) throws GeneralSecurityException, IOException {
        KeyStore store = keyStore();
        if (store.containsAlias(alias)) return (SecretKey) store.getKey(alias, null);
        if (!create) return null;
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(
            new KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setUnlockedDeviceRequired(true)
                .build()
        );
        return generator.generateKey();
    }
}
