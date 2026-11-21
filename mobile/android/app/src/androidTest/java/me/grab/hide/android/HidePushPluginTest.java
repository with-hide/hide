package me.grab.hide.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotEquals;
import static org.junit.Assert.assertTrue;

import android.Manifest;
import android.content.Context;
import android.os.SystemClock;
import android.webkit.WebView;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import com.google.firebase.messaging.FirebaseMessaging;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import org.json.JSONObject;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

/**
 * The notifications plugin as the page reaches it, through Capacitor's real bridge in the app's own activity, with the real
 * Keystore and Firebase's real token calls (the emulator's Google Play services and Google's servers): what the phone's
 * notification switch gets back when the person turns it off and on again.
 */
@RunWith(AndroidJUnit4.class)
public class HidePushPluginTest {

    private static final String ROOM = "r".repeat(43);

    /**
     * The switch turned on, then off and on again at once: the page sends the second {@code enable} without waiting for the
     * {@code forget}, as {@code web/src/mobile/appPush.ts} does, then reads the registration a later connection would send.
     */
    private static final String PAGE = """
        <!doctype html><meta charset="utf-8"><body><script>
        window.results = {};
        const push = (method) => Capacitor.nativePromise('HidePush', method, { room: '%1$s' });
        (async () => {
          results.first = (await push('enable')).token;
          const off = push('forget');
          results.again = (await push('enable')).token;
          await off;
          const now = await push('registration');
          results.found = now.found;
          results.now = now.token;
          results.done = true;
        })().catch((e) => { results.error = String(e && (e.code || e.message) || e); });
        </script>
        """.formatted(ROOM);

    private PushKeyStore store;

    @Before
    public void start() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        InstrumentationRegistry.getInstrumentation().getUiAutomation().grantRuntimePermission(context.getPackageName(), Manifest.permission.POST_NOTIFICATIONS);
        store = PushKeyStore.of(context);
        // `forget` gives the token back only when no computer's key is left, so this test's key is the only one.
        store.removeAll();
    }

    @After
    public void end() throws Exception {
        store.removeAll();
    }

    @Test
    public void anEnableSentWhileAForgetDeletesTheTokenGetsTheNextTokenNotTheDeletedOne() throws Exception {
        try (ActivityScenario<ShellActivity> scenario = ActivityScenario.launch(ShellActivity.class)) {
            AtomicReference<WebView> webView = new AtomicReference<>();
            scenario.onActivity((activity) -> {
                webView.set(activity.getBridge().getWebView());
                webView.get().loadDataWithBaseURL("https://localhost/push-plugin-test", PAGE, "text/html", "utf-8", null);
            });
            waitFor(webView.get(), "window.results.done === true || window.results.error !== undefined");
            JSONObject results = new JSONObject(new JSONObject("{\"v\":" + evaluate(webView.get(), "JSON.stringify(window.results)") + "}").getString("v"));
            assertTrue(results.toString(), results.optBoolean("done"));
            assertNotEquals("the second enable got the token the forget was deleting", results.getString("first"), results.getString("again"));
            // The computer holds the token Firebase has now, so its notices reach the phone.
            assertTrue(results.getBoolean("found"));
            assertEquals(results.getString("again"), results.getString("now"));
            // A key is held again, so Firebase keeps renewing that token: the forget turned renewal off before the enable
            // turned it on.
            assertTrue(FirebaseMessaging.getInstance().isAutoInitEnabled());

            // Off again, as the person would leave it: the token this test made is given back.
            evaluate(webView.get(), "Capacitor.nativePromise('HidePush', 'forget', { room: '" + ROOM + "' }).then(() => { results.ended = true; })");
            waitFor(webView.get(), "window.results.ended === true");
        }
    }

    private static void waitFor(WebView webView, String condition) throws Exception {
        long deadline = SystemClock.uptimeMillis() + 60_000;
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
