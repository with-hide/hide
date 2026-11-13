package me.grab.hide.android;

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

/**
 * One small secret per computer, as a file per room in a folder of the app's no-backup storage, encrypted with AES-GCM
 * under one Android Keystore key of the folder's own that never leaves the Keystore and is not backed up. Android has no
 * keychain for a small secret; this is the app's, used by the pairing store ({@link PairingStore}) and the notification
 * keys ({@link PushKeyStore}), each with its own folder and key, so one never opens the other's files.
 *
 * <p>The room is the cipher's associated data, so a file moved to another room's name does not open. A file is written
 * beside its place and moved over it, so a failed write never leaves half an item. Unlike the iOS Keychain, the folder and
 * the key go with the app when it is deleted, so nothing of an earlier install is left to clear.
 */
final class SealedFiles {

    private static final Pattern ROOM = Pattern.compile("^[A-Za-z0-9_-]{43}$");
    private static final byte VERSION = 1;
    private static final int IV_BYTES = 12;
    private static final int TAG_BITS = 128;

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
    private final String suffix;
    private final String alias;
    private final boolean unlockedOnly;

    /**
     * {@code unlockedOnly}: the key can be used only while the phone is unlocked (a pairing credential); without it, also
     * while it is locked after the first unlock since boot (a notification key, since a notice arrives on a locked phone).
     */
    SealedFiles(File folder, String suffix, String alias, boolean unlockedOnly) {
        this.folder = folder;
        this.suffix = suffix;
        this.alias = alias;
        this.unlockedOnly = unlockedOnly;
    }

    /** A computer's room, as every store names its items: 43 characters of base64url. */
    static boolean isRoom(String value) {
        return value != null && ROOM.matcher(value).matches();
    }

    boolean has(String room) {
        return file(room).exists();
    }

    void seal(String room, byte[] plain) throws Failure {
        try {
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.ENCRYPT_MODE, key(true));
            cipher.updateAAD(room.getBytes(StandardCharsets.UTF_8));
            byte[] sealed = cipher.doFinal(plain);
            byte[] iv = cipher.getIV();
            if (iv.length != IV_BYTES) throw new GeneralSecurityException("unexpected iv length");
            byte[] bytes = new byte[1 + IV_BYTES + sealed.length];
            bytes[0] = VERSION;
            System.arraycopy(iv, 0, bytes, 1, IV_BYTES);
            System.arraycopy(sealed, 0, bytes, 1 + IV_BYTES, sealed.length);
            write(file(room), bytes);
        } catch (GeneralSecurityException | IOException e) {
            throw new Failure("keychain_failed", e);
        }
    }

    /** The room's opened item, or null when it has none. A file that does not open with the key is {@code corrupt}. */
    byte[] open(String room) throws Failure {
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
            return cipher.doFinal(Arrays.copyOfRange(bytes, 1 + IV_BYTES, bytes.length));
        } catch (AEADBadTagException e) {
            throw new Failure("corrupt", e);
        } catch (GeneralSecurityException | IOException e) {
            throw new Failure("keychain_failed", e);
        }
    }

    void remove(String room) throws Failure {
        File target = file(room);
        if (target.exists() && !target.delete()) throw new Failure("keychain_failed");
    }

    /** Every item and the key. */
    void removeAll() throws Failure {
        for (String room : rooms()) remove(room);
        try {
            KeyStore store = keyStore();
            if (store.containsAlias(alias)) store.deleteEntry(alias);
        } catch (GeneralSecurityException | IOException e) {
            throw new Failure("keychain_failed", e);
        }
    }

    /**
     * The rooms that have an item, sorted. Only a folder that is not there yet holds no item. A folder that cannot be read
     * fails, as the iOS Keychain's list does: an empty answer would have the web side drop every saved computer.
     */
    List<String> rooms() throws Failure {
        List<String> rooms = new ArrayList<>();
        String[] names = folder.list();
        if (names == null) {
            if (!folder.exists()) return rooms;
            throw new Failure("keychain_failed");
        }
        for (String name : names) {
            if (!name.endsWith(suffix)) continue;
            String room = name.substring(0, name.length() - suffix.length());
            if (isRoom(room)) rooms.add(room);
        }
        Collections.sort(rooms);
        return rooms;
    }

    private File file(String room) {
        return new File(folder, room + suffix);
    }

    private void write(File target, byte[] bytes) throws IOException {
        if (!folder.isDirectory() && !folder.mkdirs()) throw new IOException("store folder not made");
        File staged = new File(folder, target.getName() + ".tmp");
        try (FileOutputStream out = new FileOutputStream(staged)) {
            out.write(bytes);
            out.getFD().sync();
        }
        if (!staged.renameTo(target)) {
            staged.delete();
            throw new IOException("store item not moved into place");
        }
    }

    private static KeyStore keyStore() throws GeneralSecurityException, IOException {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        return store;
    }

    /** The folder's key; made on the first write, null on a read before any write. */
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
                .setUnlockedDeviceRequired(unlockedOnly)
                .build()
        );
        return generator.generateKey();
    }
}
