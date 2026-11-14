package me.grab.hide.android;

import java.util.regex.Pattern;

/**
 * Which file the app's origin serves for an address, the twin of the iOS app's {@code PhoneRouter}.
 *
 * <p>Capacitor's local server is made for single-page apps: an address whose last part has no file extension answers the
 * root {@code index.html}. The phone site is not one page: its loader is the root page and every protocol bundle is a page
 * of its own at {@code /p/<n>/}, so the loader's {@code location.replace("/p/5/?host=...")} would load the loader again.
 * {@code /p/<n>/} answers that bundle's {@code index.html}; every other address keeps Capacitor's rule.
 */
public final class PhoneRouter {

    private static final Pattern BUNDLE = Pattern.compile("^/p/[1-9][0-9]*/?$");

    private PhoneRouter() {}

    /** The path of the file to serve for {@code path}, or null when Capacitor's own rule applies. */
    public static String route(String path) {
        if (path == null || !BUNDLE.matcher(path).matches()) return null;
        return (path.endsWith("/") ? path : path + "/") + "index.html";
    }
}
