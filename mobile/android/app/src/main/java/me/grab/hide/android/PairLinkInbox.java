package me.grab.hide.android;

import android.os.SystemClock;
import java.util.function.LongSupplier;

/**
 * The one pairing link waiting for the web app, the twin of the iOS app's {@code PairLinkInbox}: held in memory only and
 * for one minute, given out once (the page's {@code HideApp.takeLink}), so a retained older bundle that never asks cannot
 * leave it for a later screen to find. A link is never handed to Capacitor: {@link ShellActivity} takes it off the
 * intent before Capacitor sees the intent.
 */
public final class PairLinkInbox {

    public static final long LIFETIME_MS = 60_000;
    private static final PairLinkInbox SHARED = new PairLinkInbox(SystemClock::elapsedRealtime);

    private final LongSupplier now;
    private String link;
    private long at;
    private Runnable listener;

    public PairLinkInbox(LongSupplier now) {
        this.now = now;
    }

    public static PairLinkInbox shared() {
        return SHARED;
    }

    /** Keeps the link when it is a pairing link and tells the page one is waiting (no secret in the event); false for any other. */
    public boolean offer(String text) {
        String accepted = PairLink.accept(text);
        if (accepted == null) return false;
        Runnable notify;
        synchronized (this) {
            link = accepted;
            at = now.getAsLong();
            notify = listener;
        }
        if (notify != null) notify.run();
        return true;
    }

    /** The waiting link, once; null when none waits or it is older than a minute. */
    public synchronized String take() {
        String waiting = link;
        link = null;
        if (waiting == null || now.getAsLong() - at > LIFETIME_MS) return null;
        return waiting;
    }

    /** The one listener, {@code HideAppPlugin}'s; null removes it. */
    public synchronized void setListener(Runnable listener) {
        this.listener = listener;
    }
}
