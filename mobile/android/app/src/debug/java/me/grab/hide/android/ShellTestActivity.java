package me.grab.hide.android;

import android.net.Uri;
import android.os.Bundle;
import com.getcapacitor.CapConfig;
import java.util.ArrayList;
import java.util.List;

/**
 * The app's activity for the instrumented tests, in a debug build only: every address it would open outside the app is
 * recorded rather than handed to the phone's browser, and {@link #BREAK_LISTENER} makes Capacitor fail to add its web
 * message listener.
 */
public class ShellTestActivity extends ShellActivity {

    /**
     * A launch extra: Capacitor's config gets an origin rule the web view refuses ({@code https://exa*mple.com}), so its
     * {@code addWebMessageListener} throws on a web view that has the feature, and Capacitor falls back to a Java object.
     */
    public static final String BREAK_LISTENER = "break_listener";

    /** Read on the main thread, where the web view client adds to it. */
    public final List<String> openedOutside = new ArrayList<>();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        if (getIntent().getBooleanExtra(BREAK_LISTENER, false)) {
            config = new CapConfig.Builder(this).setAllowNavigation(new String[] { "exa*mple.com" }).create();
        }
        super.onCreate(savedInstanceState);
    }

    @Override
    protected void openOutside(Uri url) {
        openedOutside.add(url.toString());
    }
}
