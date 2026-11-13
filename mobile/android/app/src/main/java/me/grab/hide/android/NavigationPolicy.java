package me.grab.hide.android;

import java.net.URI;
import java.net.URISyntaxException;
import java.util.Locale;

/**
 * Where the web view may go, the twin of the iOS app's {@code NavigationPolicy}: one rule for a navigation and for a new
 * window ({@code window.open} and a link with a target, which the web view, making no second window, turns into a
 * navigation of its own).
 *
 * <p>Capacitor hands every address outside the app to the system ({@code Intent.ACTION_VIEW}) whatever its scheme, so a page
 * could open {@code tel:}, {@code intent:} or another app's scheme. An address inside the app's origin goes on to
 * Capacitor, which loads it; an {@code https} address goes on to Capacitor, which opens it outside; anything else is
 * cancelled.
 */
public final class NavigationPolicy {

    public enum Decision {
        /** The app's own origin, or a frame the page makes of itself: Capacitor loads it. */
        INSIDE,
        /** An https address: Capacitor opens it outside the app. */
        OUTSIDE,
        /** Every other scheme. */
        CANCEL,
    }

    private NavigationPolicy() {}

    public static Decision decide(String address) {
        URI url;
        try {
            url = new URI(address);
        } catch (URISyntaxException | NullPointerException e) {
            return Decision.CANCEL;
        }
        String scheme = url.getScheme() == null ? "" : url.getScheme().toLowerCase(Locale.ROOT);
        if (scheme.equals("about") || scheme.equals("blob")) return Decision.INSIDE;
        if (!scheme.equals("https")) return Decision.CANCEL;
        return BridgeGate.isAppOrigin(url) ? Decision.INSIDE : Decision.OUTSIDE;
    }
}
