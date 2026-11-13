package me.grab.hide.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;

import org.junit.Test;

public class PairingStoreTest {

    private static final String KEY = "A".repeat(43);

    @Test
    public void checksEachFieldOfASaveAsTheIosAppDoes() {
        assertNull(PairingStore.refusal(new PairingStore.Item(KEY, KEY, "c")));
        assertNull(PairingStore.refusal(new PairingStore.Item(KEY, KEY, null)));
        assertNull(PairingStore.refusal(new PairingStore.Item("aZ09-_" + "x".repeat(37), KEY, "x".repeat(512))));
        assertEquals("invalid_room", PairingStore.refusal(new PairingStore.Item("A".repeat(42), KEY, "c")));
        assertEquals("invalid_room", PairingStore.refusal(new PairingStore.Item("../" + "A".repeat(40), KEY, "c")));
        assertEquals("invalid_room", PairingStore.refusal(new PairingStore.Item("", KEY, "c")));
        assertEquals("invalid_field", PairingStore.refusal(new PairingStore.Item(KEY, "A".repeat(44), "c")));
        assertEquals("invalid_field", PairingStore.refusal(new PairingStore.Item(KEY, KEY, "")));
        assertEquals("invalid_field", PairingStore.refusal(new PairingStore.Item(KEY, KEY, "x".repeat(513))));
        assertEquals("invalid_field", PairingStore.refusal(new PairingStore.Item(KEY, KEY, "é".repeat(257))));
    }
}
