package me.grab.hide.android;

import java.net.URI;
import java.net.URISyntaxException;
import java.nio.charset.StandardCharsets;
import java.util.Locale;

/**
 * A Hide pairing link that arrived from outside the app (an App Link from a QR the camera app read, a link in a message)
 * or from the code scanner: the twin of the iOS app's {@code PairLink}.
 *
 * <p>The pairing secret rides in the fragment, {@code https://app.withhide.dev/#pair=<base64url>} (docs/RELAY_PROTOCOL.md).
 * Native code only checks the shape, so a link of another form never reaches the web view at all; the web app decodes it
 * with the same decoder as a scanned QR, which is what validates it, and a person presses Connect.
 */
public final class PairLink {

    public static final String HOST = "app.withhide.dev";
    public static final int MAX_BYTES = 1024;
    private static final String PREFIX = "pair=";

    private PairLink() {}

    /** The link as given when it is exactly a pairing link, null for anything else. */
    public static String accept(String text) {
        if (text == null || text.getBytes(StandardCharsets.UTF_8).length > MAX_BYTES) return null;
        URI url;
        try {
            url = new URI(text);
        } catch (URISyntaxException e) {
            return null;
        }
        String host = url.getHost();
        String path = url.getRawPath();
        String fragment = url.getRawFragment();
        if (!"https".equals(url.getScheme()) || host == null || !HOST.equals(host.toLowerCase(Locale.ROOT))) return null;
        if (url.getPort() != -1 || url.getRawUserInfo() != null || url.getRawQuery() != null) return null;
        if (path != null && !path.isEmpty() && !path.equals("/")) return null;
        if (fragment == null || !fragment.startsWith(PREFIX) || fragment.length() == PREFIX.length()) return null;
        for (int i = PREFIX.length(); i < fragment.length(); i++) {
            char c = fragment.charAt(i);
            boolean base64url = (c >= '0' && c <= '9') || (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || c == '-' || c == '_';
            if (!base64url) return null;
        }
        return text;
    }
}
