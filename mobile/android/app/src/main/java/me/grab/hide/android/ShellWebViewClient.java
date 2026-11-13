package me.grab.hide.android;

import android.net.Uri;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebView;
import com.getcapacitor.Bridge;
import com.getcapacitor.BridgeWebViewClient;
import java.net.URI;
import java.net.URISyntaxException;
import java.util.Map;

/**
 * Capacitor's web view client with the app's two rules in front of it: where the page may go ({@link NavigationPolicy})
 * and which file the app's origin serves for a protocol bundle's address ({@link PhoneRouter}). Everything else is
 * Capacitor's.
 */
final class ShellWebViewClient extends BridgeWebViewClient {

    ShellWebViewClient(Bridge bridge) {
        super(bridge);
    }

    @Override
    public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
        if (NavigationPolicy.decide(request.getUrl().toString()) == NavigationPolicy.Decision.CANCEL) return true;
        return super.shouldOverrideUrlLoading(view, request);
    }

    @Override
    public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
        Uri url = request.getUrl();
        String routed = appOrigin(url) ? PhoneRouter.route(url.getPath()) : null;
        return super.shouldInterceptRequest(view, routed == null ? request : new Routed(request, url.buildUpon().path(routed).build()));
    }

    private static boolean appOrigin(Uri url) {
        try {
            return BridgeGate.isAppOrigin(new URI(url.getScheme(), null, url.getHost(), url.getPort(), "/", null, null));
        } catch (URISyntaxException e) {
            return false;
        }
    }

    /** The same request for another file of the app's origin. */
    private static final class Routed implements WebResourceRequest {

        private final WebResourceRequest original;
        private final Uri url;

        Routed(WebResourceRequest original, Uri url) {
            this.original = original;
            this.url = url;
        }

        @Override
        public Uri getUrl() {
            return url;
        }

        @Override
        public boolean isForMainFrame() {
            return original.isForMainFrame();
        }

        @Override
        public boolean isRedirect() {
            return original.isRedirect();
        }

        @Override
        public boolean hasGesture() {
            return original.hasGesture();
        }

        @Override
        public String getMethod() {
            return original.getMethod();
        }

        @Override
        public Map<String, String> getRequestHeaders() {
            return original.getRequestHeaders();
        }
    }
}
