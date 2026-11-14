package me.grab.hide.android;

/**
 * The agent a tapped notification names, held until the page asks for it, once: the twin of the iOS app's
 * {@code PushOpenInbox}. It holds the room and the agent of the decrypted notice and nothing the computer sent besides; a
 * notification the message service could not open names none, so a tap on it opens the app and no agent.
 */
public final class PushOpenInbox {

    private static final PushOpenInbox SHARED = new PushOpenInbox();

    public static final class Target {

        public final String room;
        public final String deviceId;
        public final String paneId;

        Target(String room, String deviceId, String paneId) {
            this.room = room;
            this.deviceId = deviceId;
            this.paneId = paneId;
        }
    }

    private Target target;
    private Runnable listener;

    public static PushOpenInbox shared() {
        return SHARED;
    }

    /** Keeps the target when it names an agent of a computer by the shapes the message service writes, and tells the page one waits; false for any other. */
    public boolean offer(String room, String deviceId, String paneId) {
        if (!SealedFiles.isRoom(room) || !fits(deviceId, 256) || !fits(paneId, 512)) return false;
        Runnable notify;
        synchronized (this) {
            target = new Target(room, deviceId, paneId);
            notify = listener;
        }
        // Outside the lock: a listener that takes the target at once must not wait on it.
        if (notify != null) notify.run();
        return true;
    }

    public synchronized Target take() {
        Target waiting = target;
        target = null;
        return waiting;
    }

    /** The one listener, {@code HidePushPlugin}'s; null removes it. */
    public synchronized void setListener(Runnable listener) {
        this.listener = listener;
    }

    private static boolean fits(String value, int max) {
        return value != null && !value.isEmpty() && value.length() <= max;
    }
}
