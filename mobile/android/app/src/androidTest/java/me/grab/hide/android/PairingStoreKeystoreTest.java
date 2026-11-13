package me.grab.hide.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import android.content.Context;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.File;
import java.util.ArrayList;
import java.util.List;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

/** The pairing store on the phone's own Keystore: what a save leaves, what a later start of the app reads back. */
@RunWith(AndroidJUnit4.class)
public class PairingStoreKeystoreTest {

    private Context context;
    private PairingStore store;

    private static String room(int index) {
        return String.format("%02d", index) + "r".repeat(41);
    }

    private static final String NOISE = "n".repeat(43);

    @Before
    public void start() throws Exception {
        context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        store = PairingStore.of(context);
        store.removeAll();
    }

    @After
    public void end() throws Exception {
        store.removeAll();
    }

    private static String code(ThrowingRunnable run) {
        try {
            run.run();
        } catch (PairingStore.Failure failure) {
            return failure.code;
        }
        fail("expected a refusal");
        return null;
    }

    private interface ThrowingRunnable {
        void run() throws PairingStore.Failure;
    }

    @Test
    public void keepsAnItemAcrossANewStoreAsAfterARestartAndForgetsItOnRemove() throws Exception {
        assertNull(store.read(room(1)));
        store.save(new PairingStore.Item(room(1), NOISE, "secret-1"));
        store.save(new PairingStore.Item(room(2), NOISE, null));

        PairingStore later = PairingStore.of(context);
        assertEquals(List.of(room(1), room(2)), later.list());
        PairingStore.Item first = later.read(room(1));
        assertEquals(NOISE, first.noiseKey);
        assertEquals("secret-1", first.credential);
        assertNull(later.read(room(2)).credential);

        later.save(new PairingStore.Item(room(1), NOISE, null));
        assertNull(store.read(room(1)).credential);
        later.remove(room(1));
        later.remove(room(1));
        assertNull(store.read(room(1)));
        assertEquals(List.of(room(2)), store.list());
    }

    @Test
    public void keepsNoSecretInTheClearAndOpensAnItemOnlyUnderItsOwnRoom() throws Exception {
        store.save(new PairingStore.Item(room(1), NOISE, "plain-secret-text"));
        File folder = new File(context.getNoBackupFilesDir(), "pairing");
        File file = new File(folder, room(1) + ".pairing");
        String bytes = new String(java.nio.file.Files.readAllBytes(file.toPath()), java.nio.charset.StandardCharsets.ISO_8859_1);
        assertTrue(!bytes.contains("plain-secret-text") && !bytes.contains(NOISE));

        // An item moved to another room's name does not open there.
        assertTrue(file.renameTo(new File(folder, room(3) + ".pairing")));
        assertEquals("corrupt", code(() -> store.read(room(3))));
    }

    @Test
    public void refusesASeventeenthComputerButStillReplacesAKeptOne() throws Exception {
        List<String> rooms = new ArrayList<>();
        for (int index = 0; index < PairingStore.MAX_ITEMS; index++) {
            store.save(new PairingStore.Item(room(index), NOISE, "c"));
            rooms.add(room(index));
        }
        assertEquals("too_many", code(() -> store.save(new PairingStore.Item(room(PairingStore.MAX_ITEMS), NOISE, "c"))));
        store.save(new PairingStore.Item(room(0), NOISE, "replaced"));
        assertEquals("replaced", store.read(room(0)).credential);
        assertEquals(rooms, store.list());
    }

    @Test
    public void failsAListItCannotReadAndKeepsEveryItemForTheNextRead() throws Exception {
        File folder = new File(context.getCacheDir(), "pairing-read-test");
        File aside = new File(context.getCacheDir(), "pairing-read-test-aside");
        PairingStore other = new PairingStore(folder, PairingStore.KEY_ALIAS + ".read-test");
        try {
            // A folder not made yet holds no item; that is no failure.
            assertEquals(List.of(), other.list());
            other.save(new PairingStore.Item(room(1), NOISE, "kept"));
            // A file where the folder was cannot be listed: the list fails rather than answering that nothing is kept,
            // which would have the page drop its saved computer and then the computer's item.
            assertTrue(folder.renameTo(aside));
            assertTrue(folder.createNewFile());
            assertEquals("keychain_failed", code(() -> other.list()));
            assertEquals("keychain_failed", code(() -> other.save(new PairingStore.Item(room(2), NOISE, "c"))));
            assertTrue(folder.delete() && aside.renameTo(folder));
            assertEquals(List.of(room(1)), other.list());
            assertEquals("kept", other.read(room(1)).credential);
        } finally {
            if (folder.isFile()) folder.delete();
            if (aside.exists() && !folder.exists()) aside.renameTo(folder);
            other.removeAll();
            folder.delete();
        }
    }

    @Test
    public void refusesARoomOrAFieldOfAnotherShape() {
        assertEquals("invalid_room", code(() -> store.read("../escape")));
        assertEquals("invalid_room", code(() -> store.remove("")));
        assertEquals("invalid_field", code(() -> store.save(new PairingStore.Item(room(1), "short", "c"))));
        assertEquals("invalid_field", code(() -> store.save(new PairingStore.Item(room(1), NOISE, ""))));
    }
}
