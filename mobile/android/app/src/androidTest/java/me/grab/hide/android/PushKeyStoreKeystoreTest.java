package me.grab.hide.android;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;

import android.content.Context;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.File;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.util.Arrays;
import java.util.List;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

/**
 * The notification keys on the phone's own Keystore, in a folder and under a Keystore key of the test's own, so the app's
 * keys are left as they are.
 */
@RunWith(AndroidJUnit4.class)
public class PushKeyStoreKeystoreTest {

    private static final String LOST = "l".repeat(43);
    private static final String KEPT = "k".repeat(43);

    private File folder;
    private PushKeyStore store;

    @Before
    public void start() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        folder = new File(context.getNoBackupFilesDir(), "push-test");
        store = new PushKeyStore(folder, "me.grab.hide.android.push.test");
        store.removeAll();
    }

    @After
    public void end() throws Exception {
        store.removeAll();
    }

    @Test
    public void aKeyThatNoLongerOpensIsLeftOutOfTheTriedKeysAndReplacedByTheNextRegistration() throws Exception {
        PushKeyStore.Key lost = store.readOrCreate(LOST);
        PushKeyStore.Key kept = store.readOrCreate(KEPT);
        // Another room's sealed file in its place no longer opens there (the room is the cipher's associated data), as a
        // file whose Keystore key was lost does not.
        Files.copy(new File(folder, KEPT + ".push").toPath(), new File(folder, LOST + ".push").toPath(), StandardCopyOption.REPLACE_EXISTING);

        // The message service still tries the other computer's key.
        List<PushKeyStore.Key> tried = store.all();
        assertEquals(1, tried.size());
        assertEquals(KEPT, tried.get(0).room);
        assertArrayEquals(kept.publicKey, tried.get(0).publicKey);

        // The next `enable` for that computer answers a new key, which is kept.
        PushKeyStore.Key fresh = store.readOrCreate(LOST);
        assertFalse(Arrays.equals(lost.publicKey, fresh.publicKey));
        assertArrayEquals(fresh.publicKey, store.read(LOST).publicKey);
        assertEquals(2, store.all().size());
    }
}
