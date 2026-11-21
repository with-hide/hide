package me.grab.hide.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import android.os.SystemClock;
import android.webkit.WebView;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Predicate;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

/**
 * The app's real web view client, driven by the page's own navigations: an address of the app's host on another port
 * opens outside and the page stays, a scheme the policy cancels goes nowhere, and an address of the app's origin loads in
 * the web view. Handing the decision back to Capacitor fails the first: it compares no port and loads the address inside.
 */
@RunWith(AndroidJUnit4.class)
public class ShellNavigationWebViewTest {

    private static final String START = "https://localhost/navigation-test";
    private static final String OTHER_PORT = "https://localhost:8443/other-port";

    private ActivityScenario<ShellTestActivity> scenario;
    private WebView webView;

    @Before
    public void start() {
        scenario = ActivityScenario.launch(ShellTestActivity.class);
        AtomicReference<WebView> view = new AtomicReference<>();
        scenario.onActivity((activity) -> {
            view.set(activity.getBridge().getWebView());
            view.get().loadDataWithBaseURL(START, "<!doctype html><script>window.marker = 'start';</script>", "text/html", "utf-8", null);
        });
        webView = view.get();
    }

    @After
    public void end() {
        if (scenario != null) scenario.close();
    }

    @Test
    public void opensAnotherPortOutsideCancelsAnotherSchemeAndLoadsTheAppsOrigin() throws Exception {
        until("the start page", (page) -> page.equals("https://localhost start"));

        // The policy cancels tel:, then sends the other port outside. The navigations are decided in the order the page
        // makes them, so once the second is recorded the first has been decided too.
        navigate("tel:123");
        navigate(OTHER_PORT);
        long deadline = SystemClock.uptimeMillis() + 10_000;
        while (opened().isEmpty() && SystemClock.uptimeMillis() < deadline) {
            SystemClock.sleep(100); // wait-ok: the next look at what the client opened outside
        }
        assertEquals(List.of(OTHER_PORT), opened());
        assertEquals("https://localhost start", page());

        // An address of the app's own origin loads in the web view: the start page's marker is gone.
        navigate("https://localhost/navigation-inside");
        until("the app's origin loaded", (page) -> page.equals("https://localhost "));
        assertEquals(List.of(OTHER_PORT), opened());
    }

    private List<String> opened() {
        List<String> opened = new ArrayList<>();
        scenario.onActivity((activity) -> opened.addAll(activity.openedOutside));
        return opened;
    }

    private void navigate(String address) throws InterruptedException {
        evaluate("location.href = " + org.json.JSONObject.quote(address) + "; true");
    }

    /** The page's origin and its marker, which only the start page has. */
    private String page() throws InterruptedException {
        String raw = evaluate("location.origin + ' ' + (window.marker || '')");
        return raw == null || raw.equals("null") ? "" : raw.substring(1, raw.length() - 1);
    }

    private void until(String what, Predicate<String> reached) throws InterruptedException {
        long deadline = SystemClock.uptimeMillis() + 30_000;
        String page = page();
        while (!reached.test(page)) {
            if (SystemClock.uptimeMillis() > deadline) throw new AssertionError("never reached " + what + ": " + page);
            SystemClock.sleep(100); // wait-ok: the next look at the page
            page = page();
        }
    }

    private String evaluate(String script) throws InterruptedException {
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
