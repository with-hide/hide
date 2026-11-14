package me.grab.hide.android;

import android.annotation.SuppressLint;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.content.res.Configuration;
import android.net.Uri;
import android.os.Bundle;
import android.util.Log;
import android.webkit.WebView;
import androidx.activity.EdgeToEdge;
import androidx.activity.OnBackPressedCallback;
import androidx.core.content.ContextCompat;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsControllerCompat;
import androidx.webkit.WebViewCompat;
import androidx.webkit.WebViewFeature;
import com.getcapacitor.Bridge;
import com.getcapacitor.BridgeActivity;
import java.util.Set;

/**
 * Capacitor's activity with the app's guards put in place once the bridge exists, the twin of the iOS app's
 * {@code ShellViewController}: the gate on the bridge ({@link BridgeGate}), the one navigation rule and the bundle routing
 * ({@link ShellWebViewClient}), the pairing link taken off the intent ({@link PairLinkInbox}), the page's light or dark on
 * the system bars, the system back pressing the page's own back control, and portrait on a phone.
 *
 * <p>Room for the system bars and the keyboard is Capacitor's {@code SystemBars} plugin's (it pads the window by the bars
 * and the keyboard, or lets the page's {@code env(safe-area-inset-*)} do it on a web view that reports them); the page
 * cannot call it ({@link BridgeGate#APP_PLUGINS}).
 */
public class ShellActivity extends BridgeActivity {

    private static final String TAG = "HideShell";
    private static final String WEBVIEW_ON_PLAY = "https://play.google.com/store/apps/details?id=com.google.android.webview";

    /** The page's choice ({@code HideApp.appearance}): light, dark or the system's. */
    private String appearance = "system";

    /** The plugins a script of the app may call. A test names its own. */
    protected Set<String> exposedPlugins() {
        return BridgeGate.APP_PLUGINS;
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        setIntent(withoutPairLink(getIntent()));
        registerPlugin(HideKeychainPlugin.class);
        registerPlugin(HideScannerPlugin.class);
        registerPlugin(HideAppPlugin.class);
        registerPlugin(HidePushPlugin.class);
        super.onCreate(savedInstanceState);
        EdgeToEdge.enable(this);
        // A phone is held upright; a tablet or an unfolded phone turns as it likes (Android 16 ignores the lock there anyway).
        if (getResources().getConfiguration().smallestScreenWidthDp < 600) setRequestedOrientation(ActivityInfo.SCREEN_ORIENTATION_PORTRAIT);
        applyAppearance();
    }

    @SuppressLint("RequiresFeature")
    @Override
    protected void load() {
        // Without the web message listener Capacitor would answer every frame through `addJavascriptInterface`: no bridge
        // is made at all, and the page asks for a newer web view instead.
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
            showUpdatePage();
            return;
        }
        super.load();
        // With the feature, Capacitor still falls back to that Java object when it cannot add its listener; the web view
        // refused it (GatedWebView), and the app drops the bridge rather than run one the page cannot reach or ungated.
        if (((GatedWebView) bridge.getWebView()).refused(BridgeGate.HANDLER)) {
            Log.w(TAG, "the bridge's web message listener could not be added; no bridge");
            dropBridge();
            return;
        }
        BridgeGate.install(bridge, exposedPlugins());
        bridge.setWebViewClient(new ShellWebViewClient(bridge, this::openOutside));
        WebView webView = bridge.getWebView();
        seedTheme(webView);
        getOnBackPressedDispatcher().addCallback(this, new OnBackPressedCallback(true) {
            @Override
            public void handleOnBackPressed() {
                pressPageBack();
            }
        });
        DebugInputs.attach(this, bridge, getIntent());
        // Capacitor began loading the page under its own listener, and a page keeps the listener it was loaded with: every
        // page the app shows is loaded after the gate took the listener's place.
        webView.stopLoading();
        bridge.reload();
    }

    private void showUpdatePage() {
        setContentView(R.layout.update_webview);
        findViewById(R.id.update_webview_open).setOnClickListener((view) -> openOutside(Uri.parse(WEBVIEW_ON_PLAY)));
    }

    /**
     * The bridge Capacitor made goes, with its web view and plugins, and the activity goes on as on a web view without the
     * listener feature: Capacitor's lifecycle steps skip a missing bridge.
     */
    private void dropBridge() {
        Bridge made = bridge;
        bridge = null;
        made.getWebView().stopLoading();
        showUpdatePage();
        made.onDestroy();
        made.onDetachedFromWindow();
    }

    /** An address the page may not load but the phone may open goes to the phone's browser. A test records it instead. */
    protected void openOutside(Uri url) {
        try {
            startActivity(new Intent(Intent.ACTION_VIEW, url));
        } catch (ActivityNotFoundException e) {
            // No app takes https on this phone; the page stays where it is.
            Log.w(TAG, "no app opens an outside address");
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(withoutPairLink(intent));
    }

    /**
     * A pairing link goes to the inbox and the intent goes on without it, so Capacitor (whose bridge keeps the launch
     * address for its whole life) never holds the secret.
     */
    private static Intent withoutPairLink(Intent intent) {
        if (intent == null || !Intent.ACTION_VIEW.equals(intent.getAction()) || intent.getData() == null) return intent;
        if (!PairLinkInbox.shared().offer(intent.getData().toString())) return intent;
        Intent rest = new Intent(intent);
        rest.setData(null);
        return rest;
    }

    @Override
    public void onConfigurationChanged(Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        // Capacitor's SystemBars puts the system's own style back on every change; the page's choice goes over it.
        applyAppearance();
    }

    void setAppearance(String scheme) {
        appearance = scheme;
        applyAppearance();
    }

    private boolean dark() {
        if (!appearance.equals("system")) return appearance.equals("dark");
        return (getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES;
    }

    /** The system bars' icons and the color behind them and the web view follow the page's light or dark. */
    private void applyAppearance() {
        boolean dark = dark();
        int background = ContextCompat.getColor(this, dark ? R.color.launch_background_dark : R.color.launch_background_light);
        WindowInsetsControllerCompat bars = WindowCompat.getInsetsController(getWindow(), getWindow().getDecorView());
        bars.setAppearanceLightStatusBars(!dark);
        bars.setAppearanceLightNavigationBars(!dark);
        getWindow().getDecorView().setBackgroundColor(background);
        if (bridge != null) bridge.getWebView().setBackgroundColor(background);
    }

    /**
     * Styles can paint before the module that follows the page's theme runs: the stored choice is put on the document at
     * its start, each protocol navigation included, as the iOS app does. settings.ts owns every later change.
     */
    @SuppressLint("RequiresFeature")
    private static void seedTheme(WebView webView) {
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) return;
        WebViewCompat.addDocumentStartJavaScript(webView, """
            (() => {
              let theme;
              try { theme = JSON.parse(localStorage.getItem('hide.phone.settings'))?.theme; } catch {}
              const dark = theme === 'dark' || (theme !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches);
              const apply = () => {
                if (!document.documentElement) return false;
                document.documentElement.classList.toggle('dark', dark);
                return true;
              };
              if (!apply()) {
                const observer = new MutationObserver(() => { if (apply()) observer.disconnect(); });
                observer.observe(document, { childList: true });
              }
            })();
            """, Set.of(BridgeGate.ORIGIN));
    }

    /**
     * Detail screens are page state, not web view history: the system back presses the page's own visible back control,
     * within the top sheet when one is open, so a mission, a View and the start pages keep their owner and action. An open
     * sheet with no back control closes as Escape closes it. With nothing to go back to, the app goes to the background.
     */
    private void pressPageBack() {
        bridge.getWebView().evaluateJavascript("""
            (() => {
              const dialog = [...document.querySelectorAll('[role="dialog"][data-state="open"]')].pop();
              const back = (dialog || document).querySelector('button[data-phone-back]:not(:disabled)');
              if (back && back.getClientRects().length) {
                document.activeElement?.blur();
                back.click();
                return true;
              }
              if (!dialog) return false;
              dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
              return true;
            })()
            """, (result) -> {
                if (!"true".equals(result)) moveTaskToBack(true);
            });
    }
}
