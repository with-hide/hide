package me.grab.hide.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import org.junit.Test;

public class PairLinkTest {

    private static final String LINK = "https://app.withhide.dev/#pair=AbC-d_09";

    @Test
    public void acceptsExactlyThePairingLinkOfTheHostedSite() {
        assertEquals(LINK, PairLink.accept(LINK));
        assertEquals("https://APP.withhide.dev#pair=x", PairLink.accept("https://APP.withhide.dev#pair=x"));
    }

    @Test
    public void refusesEveryOtherShape() {
        String[] refused = {
            null,
            "",
            "http://app.withhide.dev/#pair=abc",
            "HTTPS://app.withhide.dev/#pair=abc",
            "https://app.withhide.dev.evil.example/#pair=abc",
            "https://evil.example/#pair=abc",
            "https://app.withhide.dev:443/#pair=abc",
            "https://me@app.withhide.dev/#pair=abc",
            "https://app.withhide.dev/?x=1#pair=abc",
            "https://app.withhide.dev/p/5/#pair=abc",
            "https://app.withhide.dev/#pair=",
            "https://app.withhide.dev/#pairing=abc",
            "https://app.withhide.dev/#pair=a%20b",
            "https://app.withhide.dev/#pair=a.b",
            "https://app.withhide.dev/",
            "intent://app.withhide.dev/#pair=abc",
        };
        for (String text : refused) assertNull(text, PairLink.accept(text));
    }

    @Test
    public void refusesALinkLongerThanTheLimit() {
        String prefix = "https://app.withhide.dev/#pair=";
        String fits = prefix + "a".repeat(PairLink.MAX_BYTES - prefix.length());
        assertEquals(fits, PairLink.accept(fits));
        assertNull(PairLink.accept(fits + "a"));
    }

    @Test
    public void theInboxGivesALinkOutOnceAndOnlyWithinAMinute() {
        AtomicLong clock = new AtomicLong(1_000);
        AtomicInteger told = new AtomicInteger();
        PairLinkInbox inbox = new PairLinkInbox(clock::get);
        inbox.setListener(told::incrementAndGet);

        assertFalse(inbox.offer("https://evil.example/#pair=abc"));
        assertEquals(0, told.get());
        assertTrue(inbox.offer(LINK));
        assertEquals(1, told.get());
        assertEquals(LINK, inbox.take());
        assertNull(inbox.take());

        assertTrue(inbox.offer(LINK));
        clock.addAndGet(PairLinkInbox.LIFETIME_MS + 1);
        assertNull(inbox.take());
    }
}
