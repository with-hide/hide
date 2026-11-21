package me.grab.hide.android;

import android.annotation.SuppressLint;
import android.net.Uri;
import android.util.Log;
import android.webkit.WebView;
import androidx.webkit.JavaScriptReplyProxy;
import androidx.webkit.WebMessageCompat;
import androidx.webkit.WebViewCompat;
import com.getcapacitor.Bridge;
import com.getcapacitor.MessageHandler;
import java.lang.reflect.Field;
import java.net.URI;
import java.util.Locale;
import java.util.Set;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * The only thing that may reach Capacitor's bridge from the web view, the twin of the iOS app's {@code BridgeGate}.
 *
 * <p>Capacitor 8.6 posts the page's calls through a web message listener named {@code androidBridge}, which answers only
 * the main frame, but it falls back to {@code addJavascriptInterface}, which answers every frame, when the web view lacks
 * that feature or the listener cannot be added; it loads any plugin a call names, its own {@code WebView} (which moves the
 * folder the app is served from) and {@code CapacitorHttp} (which reaches any address past the page's content policy)
 * among them; and two of its plugins put a Java object of their own into every frame. The phone app draws agent-made
 * HTML in a sandboxed frame ({@code look-frame.html}). The gate takes the listener's place under the same name, for the
 * app's own origin only, and passes a call on only when it comes from the main frame, is a plugin call or an error
 * report, and names a plugin the app has chosen to expose. The app's web view refuses every Java object
 * ({@link GatedWebView}), and a web view without the listener feature, or one where Capacitor could not add its listener,
 * never gets a bridge at all ({@link ShellActivity}).
 */
public final class BridgeGate implements WebViewCompat.WebMessageListener {

    public static final String HANDLER = "androidBridge";

    /** The origin the app serves its bundle from: Capacitor's {@code https://localhost}. */
    public static final String ORIGIN = "https://localhost";

    /**
     * The plugins the web side may call: the app's own four ({@code @CapacitorPlugin(name = ...)}). A plugin is added
     * here in the same change that links it; {@code mobile/scripts/check-versions.mjs} compares this list with the plugin
     * classes of the app and fails when they differ. Unlike iOS there is no {@code Console}: Capacitor's Android bridge
     * hands the page's console to the system log without a plugin.
     */
    public static final Set<String> APP_PLUGINS = Set.of("HideKeychain", "HideScanner", "HideApp", "HidePush");

    private static final String TAG = "HideBridge";

    private final MessageHandler inner;
    private final Set<String> plugins;

    BridgeGate(MessageHandler inner, Set<String> plugins) {
        this.inner = inner;
        this.plugins = plugins;
    }

    /** True for an address of the app's own origin: https, host localhost, no port. */
    public static boolean isAppOrigin(URI url) {
        return (
            "https".equalsIgnoreCase(url.getScheme()) &&
            url.getHost() != null &&
            url.getHost().toLowerCase(Locale.ROOT).equals("localhost") &&
            url.getPort() == -1
        );
    }

    /**
     * The decision, apart from the web view so a test can state every case: null to pass the call on, else why it is
     * refused. {@code type} and {@code pluginId} are the call's own fields, null when absent; {@code malformed} when the
     * call was not a JSON object. Capacitor's Android script sends a plugin call with no type, an error report as
     * {@code js.error}, and a Cordova call as {@code cordova}.
     */
    public static String verdict(boolean isMainFrame, String origin, boolean malformed, String type, String pluginId, Set<String> plugins) {
        if (!isMainFrame) return "not_main_frame";
        if (!ORIGIN.equals(origin)) return "foreign_origin";
        if (malformed) return "malformed";
        if (type == null) return pluginId != null && plugins.contains(pluginId) ? null : "plugin_not_exposed";
        // Every other type, `cordova` included: the app links no Cordova plugin.
        return type.equals("js.error") ? null : "call_type";
    }

    /**
     * Puts the gate in place of what Capacitor registered on the bridge's web view. Throws when it cannot: an unguarded
     * bridge must not run.
     */
    @SuppressLint("RequiresFeature")
    public static BridgeGate install(Bridge bridge, Set<String> plugins) {
        WebView webView = bridge.getWebView();
        BridgeGate gate = new BridgeGate(capacitorHandler(bridge), plugins);
        WebViewCompat.removeWebMessageListener(webView, HANDLER);
        WebViewCompat.addWebMessageListener(webView, HANDLER, Set.of(ORIGIN), gate);
        return gate;
    }

    /**
     * Capacitor's own message handler, which runs a passed call and answers it. Bridge keeps it in a private field and
     * names no way to it; Capacitor is pinned to one version (check-versions.mjs) and the instrumented gate test fails if
     * this read stops finding it.
     */
    private static MessageHandler capacitorHandler(Bridge bridge) {
        try {
            Field field = Bridge.class.getDeclaredField("msgHandler");
            field.setAccessible(true);
            return (MessageHandler) field.get(bridge);
        } catch (ReflectiveOperationException | ClassCastException e) {
            throw new IllegalStateException("Capacitor's bridge has no message handler to gate", e);
        }
    }

    @Override
    public void onPostMessage(WebView view, WebMessageCompat message, Uri sourceOrigin, boolean isMainFrame, JavaScriptReplyProxy replyProxy) {
        String data = message.getType() == WebMessageCompat.TYPE_STRING ? message.getData() : null;
        boolean malformed = data == null;
        String type = null;
        String pluginId = null;
        if (!malformed) {
            try {
                JSONObject call = new JSONObject(data);
                type = call.has("type") ? call.optString("type", null) : null;
                pluginId = call.has("pluginId") ? call.optString("pluginId", null) : null;
            } catch (JSONException e) {
                malformed = true;
            }
        }
        String reason = verdict(isMainFrame, sourceOrigin.toString(), malformed, type, pluginId, plugins);
        if (reason == null) {
            // The answer goes back through Capacitor's own path into the main frame (`Capacitor.fromNative`).
            inner.postMessage(data);
        } else {
            // The reason and the frame only; never the call's body, which can carry a credential.
            Log.w(TAG, "bridge call refused: " + reason + " mainFrame=" + isMainFrame);
        }
    }
}
