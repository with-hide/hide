package me.grab.hide.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import android.content.Intent;
import android.os.SystemClock;
import android.webkit.CookieManager;
import android.webkit.WebView;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import androidx.webkit.WebViewFeature;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import org.json.JSONObject;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

/**
 * The app's own activity, with its real web view and Capacitor's real bridge: a call from a sandboxed frame or from a
 * same-origin frame never reaches a plugin, the main frame's call to one of the app's plugins does, and the main frame's
 * call to a plugin of Capacitor's own (CapacitorCookies, which sets a cookie) does not. Without {@link BridgeGate}'s
 * install line in {@link ShellActivity} this fails: Capacitor answers the cookie call. When Capacitor cannot add its
 * listener, the app makes no bridge; without that check it runs with Capacitor's fallback in place.
 */
@RunWith(AndroidJUnit4.class)
public class BridgeGateWebViewTest {

    private static final String MAIN = "M".repeat(43);
    private static final String SAME = "S".repeat(43);
    private static final String SANDBOX = "X".repeat(43);
    private static final String NOISE = "N".repeat(43);

    /**
     * Each frame posts a plugin call the way Capacitor's script does and tells the page what the web view put into it;
     * the main frame then saves its own item. The bridge takes calls in order, so once that last answer is in, every call
     * before it has been decided.
     */
    private static final String PAGE = """
        <!doctype html><meta charset="utf-8"><body><script>
        window.results = { frames: {} };
        const post = (room) => JSON.stringify({ callbackId: '-1', pluginId: 'HideKeychain', methodName: 'save', options: { room, noiseKey: '%2$s' } });
        const FRAME = (room) => `<script>
          const seen = { bridge: typeof window.androidBridge, cookies: typeof window.CapacitorCookiesAndroidInterface };
          try { window.androidBridge.postMessage(${JSON.stringify(post(room))}); } catch (e) {}
          try { window.CapacitorCookiesAndroidInterface.setCookie('https://localhost', 'gateframe=${room}'); } catch (e) {}
          parent.postMessage({ room: '${room}', seen }, '*');
        <\\/script>`;
        window.addEventListener('message', async (event) => {
          results.frames[event.data.room] = event.data.seen;
          if (Object.keys(results.frames).length < 2) return;
          await Capacitor.nativePromise('HideKeychain', 'save', { room: '%1$s', noiseKey: '%2$s' });
          results.done = true;
        });
        Capacitor.nativePromise('HideApp', 'info').then((info) => { results.info = info; });
        Capacitor.nativePromise('CapacitorCookies', 'setCookie', { url: 'https://localhost', key: 'gatemain', value: 'main' })
          .then(() => { results.cookie = 'answered'; }, () => { results.cookie = 'rejected'; });
        for (const [room, sandbox] of [['%3$s', false], ['%4$s', true]]) {
          const frame = document.createElement('iframe');
          if (sandbox) frame.setAttribute('sandbox', 'allow-scripts');
          frame.srcdoc = FRAME(room);
          document.body.append(frame);
        }
        </script>
        """.formatted(MAIN, NOISE, SAME, SANDBOX);

    private ActivityScenario<ShellActivity> scenario;
    private PairingStore store;

    @Before
    public void start() throws Exception {
        store = PairingStore.of(InstrumentationRegistry.getInstrumentation().getTargetContext());
        store.removeAll();
        clearCookies();
        scenario = ActivityScenario.launch(ShellActivity.class);
    }

    @After
    public void end() throws Exception {
        if (scenario != null) scenario.close();
        store.removeAll();
        clearCookies();
    }

    @Test
    public void onlyTheMainFramesCallToAnAppPluginReachesTheBridge() throws Exception {
        AtomicReference<WebView> webView = new AtomicReference<>();
        scenario.onActivity((activity) -> {
            webView.set(activity.getBridge().getWebView());
            // The app's web view, which takes no Java object (GatedWebView), not Capacitor's.
            assertTrue(webView.get() instanceof GatedWebView);
            webView.get().loadDataWithBaseURL("https://localhost/gate-test", PAGE, "text/html", "utf-8", null);
        });
        JSONObject results = waitForDone(webView.get());

        // The page's frames made their calls: the same-origin one through the bridge object the web view put into it.
        JSONObject same = results.getJSONObject("frames").getJSONObject(SAME);
        JSONObject sandboxed = results.getJSONObject("frames").getJSONObject(SANDBOX);
        assertEquals("object", same.getString("bridge"));
        assertEquals("undefined", sandboxed.getString("bridge"));
        // Capacitor's cookie object is in no frame.
        assertEquals("undefined", same.getString("cookies"));
        assertEquals("undefined", sandboxed.getString("cookies"));

        // Only the main frame's save reached the store; the main frame's own call reached HideApp.
        assertEquals(List.of(MAIN), store.list());
        assertTrue(results.getJSONObject("info").getString("version").matches("[0-9]+\\.[0-9]+\\.[0-9]+"));
        // Capacitor's own plugin was never called, and no frame set a cookie.
        assertFalse(results.has("cookie"));
        String cookies = CookieManager.getInstance().getCookie("https://localhost");
        assertFalse(String.valueOf(cookies), cookies != null && cookies.contains("gate"));
    }

    @Test
    public void makesNoBridgeWhenCapacitorCannotAddItsListener() {
        // The web view has the listener feature, so only the failed registration leads here.
        assertTrue(WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER));
        Intent intent = new Intent(InstrumentationRegistry.getInstrumentation().getTargetContext(), ShellTestActivity.class)
            .putExtra(ShellTestActivity.BREAK_LISTENER, true);
        try (ActivityScenario<ShellTestActivity> broken = ActivityScenario.launch(intent)) {
            broken.onActivity((activity) -> {
                assertNull(activity.getBridge());
                assertTrue(activity.findViewById(R.id.update_webview_title) != null);
                assertNull(activity.findViewById(R.id.webview));
            });
        }
    }

    /** Polls the page's own record until its last call is answered. */
    private static JSONObject waitForDone(WebView webView) throws Exception {
        long deadline = SystemClock.uptimeMillis() + 30_000;
        while (SystemClock.uptimeMillis() < deadline) {
            String raw = evaluate(webView, "JSON.stringify(window.results || null)");
            if (raw != null && !raw.equals("null") && !raw.equals("\"null\"")) {
                JSONObject results = new JSONObject(new JSONObject("{\"v\":" + raw + "}").getString("v"));
                if (results.optBoolean("done")) return results;
            }
            SystemClock.sleep(100); // wait-ok: the next poll of the page's record
        }
        throw new AssertionError("the page's last call was never answered: " + evaluate(webView, "JSON.stringify(window.results || null)"));
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

    private static void clearCookies() throws InterruptedException {
        CountDownLatch done = new CountDownLatch(1);
        InstrumentationRegistry.getInstrumentation().runOnMainSync(() -> CookieManager.getInstance().removeAllCookies((removed) -> done.countDown()));
        assertTrue(done.await(10, TimeUnit.SECONDS));
    }
}
