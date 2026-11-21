package me.grab.hide.android;

import android.annotation.SuppressLint;
import android.content.Intent;
import android.os.Handler;
import android.os.Looper;
import android.webkit.WebView;
import androidx.webkit.WebViewCompat;
import androidx.webkit.WebViewFeature;
import com.getcapacitor.Bridge;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;
import org.json.JSONObject;

/**
 * The emulator check's inputs, in a debug build only (src/release has an empty twin): nothing outside can tap the web
 * view, and `build-android.sh run` drives the app with launch extras, as the iOS simulator check drives its app with
 * launch values. The pairing link itself arrives as a real App Link intent, through the same inbox a camera's link
 * reaches.
 *
 * <ul>
 *   <li>{@code hide_debug_connect}: once the page shows the pairing screen of a link, Connect is pressed.</li>
 *   <li>{@code hide_debug_taps}: selectors separated by {@code ;;}, each pressed once the page has it, in order; one written
 *       {@code <selector>::=<text>} is filled with that text, one written {@code <selector>::^} is scrolled to the top. The
 *       file {@code hide-debug-taps} in the app's cache folder says how it went: {@code done} once the last one is
 *       pressed, {@code failed <selector>} when one is never found.</li>
 *   <li>{@code hide_debug_now}: a local date and time ({@code 2026-11-09T09:41:00}) the page's clock starts from.</li>
 * </ul>
 */
final class DebugInputs {

    private static final int ATTEMPTS = 120;

    private DebugInputs() {}

    static void attach(ShellActivity activity, Bridge bridge, Intent intent) {
        if (intent == null) return;
        WebView webView = bridge.getWebView();
        File marker = new File(activity.getCacheDir(), "hide-debug-taps");
        marker.delete();
        String now = intent.getStringExtra("hide_debug_now");
        if (now != null && !now.isEmpty()) startClock(webView, now);
        Handler main = new Handler(Looper.getMainLooper());
        if (intent.getBooleanExtra("hide_debug_connect", false)) main.postDelayed(() -> pressConnect(webView, main, 0), 3000);
        String taps = intent.getStringExtra("hide_debug_taps");
        if (taps != null && !taps.isEmpty()) {
            List<String> steps = new ArrayList<>();
            for (String step : taps.split(";;")) if (!step.trim().isEmpty()) steps.add(step.trim());
            main.postDelayed(() -> tap(webView, main, marker, steps, 0), 3000);
        }
    }

    /** Starts the page's clock at the given time and lets it run on from there, before any page script. */
    @SuppressLint("RequiresFeature")
    private static void startClock(WebView webView, String now) {
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) return;
        String script = """
            (() => {
              const Real = Date, base = new Real(%s).getTime(), began = Real.now();
              if (Number.isNaN(base)) return;
              const now = () => base + (Real.now() - began);
              Date = new Proxy(Real, {
                construct: (target, args, newTarget) => Reflect.construct(target, args.length === 0 ? [now()] : args, newTarget),
                get: (target, key, receiver) => (key === 'now' ? now : Reflect.get(target, key, receiver)),
              });
            })();
            """.formatted(JSONObject.quote(now));
        WebViewCompat.addDocumentStartJavaScript(webView, script, Set.of(BridgeGate.ORIGIN));
    }

    private static void tap(WebView webView, Handler main, File marker, List<String> steps, int attempt) {
        if (steps.isEmpty()) return;
        String step = steps.get(0);
        if (attempt >= ATTEMPTS) {
            report(marker, "failed " + step);
            return;
        }
        boolean scrolls = step.endsWith("::^");
        String[] parts = (scrolls ? step.substring(0, step.length() - 3) : step).split("::=", 2);
        String selector = parts[0].trim();
        // A press is a mouse-down and a click: the page's tab strip reacts to the first, its buttons to the second.
        String action = "found.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true, button: 0 })); found.click();";
        if (scrolls) {
            action = "found.scrollIntoView({ block: 'start' });";
        } else if (parts.length > 1) {
            // A field is filled the way typing does: through the element's own setter, then an input event.
            action = "Object.getOwnPropertyDescriptor(Object.getPrototypeOf(found), 'value').set.call(found, " + JSONObject.quote(parts[1]) +
                "); found.dispatchEvent(new Event('input', { bubbles: true }));";
        }
        String script = "(() => { const found = document.querySelector(" + JSONObject.quote(selector) + "); if (!found) return false; " + action + " return true; })()";
        webView.evaluateJavascript(script, (result) -> {
            boolean pressed = "true".equals(result);
            if (pressed && steps.size() == 1) report(marker, "done");
            List<String> rest = pressed ? steps.subList(1, steps.size()) : steps;
            main.postDelayed(() -> tap(webView, main, marker, rest, pressed ? 0 : attempt + 1), pressed ? 1500 : 500);
        });
    }

    private static void pressConnect(WebView webView, Handler main, int attempt) {
        String script = """
            (() => {
              const button = [...document.querySelectorAll("button")].find((candidate) => candidate.textContent.trim() === "Connect");
              if (!button) return false;
              button.click();
              return true;
            })()
            """;
        webView.evaluateJavascript(script, (result) -> {
            if ("true".equals(result) || attempt >= ATTEMPTS) return;
            main.postDelayed(() -> pressConnect(webView, main, attempt + 1), 500);
        });
    }

    private static void report(File marker, String outcome) {
        try (FileOutputStream out = new FileOutputStream(marker)) {
            out.write(outcome.getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {
            // The script that waits on the file fails on its own when the word never comes.
        }
    }
}
