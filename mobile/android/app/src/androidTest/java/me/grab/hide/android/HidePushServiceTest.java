package me.grab.hide.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationManager;
import android.content.Context;
import android.os.SystemClock;
import android.service.notification.StatusBarNotification;
import android.webkit.WebView;
import androidx.lifecycle.Lifecycle;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import com.google.firebase.messaging.RemoteMessage;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Predicate;
import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

/**
 * The message service in the app's own process with the real Keystore and the system's notification manager: an FCM data
 * message made the way hided and the relay make one is handed to the service, the notification it shows is read back from
 * the system, and a tap on it (its own content intent) brings the app forward with the agent waiting for the page's
 * {@code HidePush.takeOpen}, which a page on the app's origin calls through Capacitor's real bridge.
 */
@RunWith(AndroidJUnit4.class)
public class HidePushServiceTest {

    /** Rooms no real computer has, so the app's own keys are left as they are. */
    private static final String ROOM = "p".repeat(43);
    private static final String OTHER = "q".repeat(43);

    /** The page the tap reaches: it listens as the phone app does and records the agent `takeOpen` answers. */
    private static final String PAGE = """
        <!doctype html><meta charset="utf-8"><body><script>
        window.results = { opened: [] };
        Promise.resolve(Capacitor.addListener('HidePush', 'open', async () => {
          results.opened.push(await Capacitor.nativePromise('HidePush', 'takeOpen'));
        })).then(() => { results.listening = true; });
        </script>
        """;

    /** The service as Android runs it, with the app's context. */
    static final class Attached extends HidePushService {
        Attached(Context context) {
            attachBaseContext(context);
        }
    }

    private Context context;
    private PushKeyStore store;
    private NotificationManager manager;

    @Before
    public void start() throws Exception {
        context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        InstrumentationRegistry.getInstrumentation().getUiAutomation().grantRuntimePermission(context.getPackageName(), Manifest.permission.POST_NOTIFICATIONS);
        store = PushKeyStore.of(context);
        store.remove(ROOM);
        store.remove(OTHER);
        manager = context.getSystemService(NotificationManager.class);
        manager.cancelAll();
        PushOpenInbox.shared().take();
    }

    @After
    public void end() throws Exception {
        store.remove(ROOM);
        store.remove(OTHER);
        manager.cancelAll();
        PushOpenInbox.shared().take();
    }

    private static RemoteMessage message(String hide) {
        return new RemoteMessage.Builder("relay@fcm.googleapis.com").addData("hide", hide).build();
    }

    private static String seal(JSONObject notice, PushKeyStore.Key key) throws Exception {
        return NoticeSealer.seal(notice.toString().getBytes(StandardCharsets.UTF_8), key.publicKey, key.auth);
    }

    /** The app's notifications once the system lists what {@code until} waits for; the manager posts asynchronously. */
    private List<StatusBarNotification> shown(Predicate<List<StatusBarNotification>> until) {
        long deadline = SystemClock.uptimeMillis() + 10_000;
        List<StatusBarNotification> active = List.of();
        while (SystemClock.uptimeMillis() < deadline) {
            active = List.of(manager.getActiveNotifications());
            if (until.test(active)) return active;
            SystemClock.sleep(100); // wait-ok: the next read of the system's list
        }
        throw new AssertionError("the system never listed the expected notifications: " + tags(active));
    }

    private static List<String> tags(List<StatusBarNotification> active) {
        List<String> tags = new ArrayList<>();
        for (StatusBarNotification notification : active) tags.add(notification.getTag());
        return tags;
    }

    private static StatusBarNotification tagged(List<StatusBarNotification> active, String tag) {
        for (StatusBarNotification notification : active) if (tag.equals(notification.getTag())) return notification;
        return null;
    }

    @Test
    public void aNoticeForAHeldKeyReplacesItsAgentsLastClosesWhatItNamesAndATapHandsTheAgentToThePage() throws Exception {
        PushKeyStore.Key key = store.readOrCreate(ROOM);
        Attached service = new Attached(context);
        // The notice that is closed, the agent's earlier notice, then the new one, which names the first to close.
        service.onMessageReceived(message(seal(NoticeSealer.notice(ROOM).put("tag", "dev|w1:p1").put("pane_id", "w1:p1").put("clear", new JSONArray()), key)));
        service.onMessageReceived(message(seal(NoticeSealer.notice(ROOM).put("title", "An older notice").put("clear", new JSONArray()), key)));
        shown((active) -> active.size() == 2);
        service.onMessageReceived(message(seal(NoticeSealer.notice(ROOM), key)));
        // One notification is left, the agent's, with the new notice's title: the system lists a close and an update in
        // the order they were made, so both are waited for.
        List<StatusBarNotification> active = shown((list) -> list.size() == 1 && tagged(list, "dev|w1:p2") != null
            && "Fix the build".equals(tagged(list, "dev|w1:p2").getNotification().extras.getString(Notification.EXTRA_TITLE)));
        StatusBarNotification notice = tagged(active, "dev|w1:p2");
        String body = String.valueOf(notice.getNotification().extras.getCharSequence(Notification.EXTRA_TEXT));
        assertTrue(body, body.contains("Should I update the lockfile too?") && body.endsWith(" · contong"));
        assertEquals(Notices.CHANNEL, notice.getNotification().getChannelId());

        try (ActivityScenario<ShellActivity> scenario = ActivityScenario.launch(ShellActivity.class)) {
            AtomicReference<WebView> webView = new AtomicReference<>();
            scenario.onActivity((activity) -> {
                webView.set(activity.getBridge().getWebView());
                webView.get().loadDataWithBaseURL("https://localhost/push-test", PAGE, "text/html", "utf-8", null);
            });
            waitFor(webView.get(), "window.results && window.results.listening === true");
            notice.getNotification().contentIntent.send();
            waitFor(webView.get(), "window.results.opened.length > 0");
            JSONArray opened = new JSONArray(new JSONObject("{\"v\":" + evaluate(webView.get(), "JSON.stringify(window.results.opened)") + "}").getString("v"));
            assertEquals(1, opened.length());
            JSONObject target = opened.getJSONObject(0);
            assertEquals(ROOM, target.getString("room"));
            assertEquals("dev", target.getString("device_id"));
            assertEquals("w1:p2", target.getString("pane_id"));
            assertEquals(Lifecycle.State.RESUMED, scenario.getState());
            assertNull("given out once", PushOpenInbox.shared().take());
        }
    }

    @Test
    public void aBodyNoKeyOpensShowsTheFixedTextOnce() throws Exception {
        PushKeyStore.Key gone = store.readOrCreate(ROOM);
        PushKeyStore.Key other = store.readOrCreate(OTHER);
        String forRoom = seal(NoticeSealer.notice(ROOM), gone);
        // The key goes, as when its computer is removed: its notices open no more.
        store.remove(ROOM);
        Attached service = new Attached(context);
        service.onMessageReceived(message(forRoom));
        service.onMessageReceived(message("not a body"));
        // A notice that names a room other than the key's computer's stays closed too.
        service.onMessageReceived(message(seal(NoticeSealer.notice(ROOM), other)));
        // The system lists notifications in the order they were posted: once this last one is listed, so is every other.
        service.onMessageReceived(message(seal(NoticeSealer.notice(OTHER).put("tag", "last"), other)));
        List<StatusBarNotification> active = shown((list) -> tagged(list, "last") != null);
        List<String> listed = tags(active);
        listed.sort(null);
        assertEquals(List.of(Notices.FIXED_TAG, "last"), listed);
        Notification fixed = tagged(active, Notices.FIXED_TAG).getNotification();
        assertEquals(context.getString(R.string.app_name), fixed.extras.getString(Notification.EXTRA_TITLE));
        assertEquals(context.getString(R.string.notice_fixed), String.valueOf(fixed.extras.getCharSequence(Notification.EXTRA_TEXT)));
    }

    private static void waitFor(WebView webView, String condition) throws Exception {
        long deadline = SystemClock.uptimeMillis() + 30_000;
        while (SystemClock.uptimeMillis() < deadline) {
            if ("true".equals(evaluate(webView, "Boolean(" + condition + ")"))) return;
            SystemClock.sleep(100); // wait-ok: the next poll of the page's record
        }
        throw new AssertionError("the page never reached " + condition + ": " + evaluate(webView, "JSON.stringify(window.results || null)"));
    }

    private static String evaluate(WebView webView, String script) throws InterruptedException {
        CountDownLatch done = new CountDownLatch(1);
        AtomicReference<String> value = new AtomicReference<>();
        InstrumentationRegistry.getInstrumentation().runOnMainSync(() -> webView.evaluateJavascript(script, (result) -> {
            value.set(result);
            done.countDown();
        }));
        assertTrue("the web view did not answer", done.await(10, TimeUnit.SECONDS));
        return value.get();
    }
}
