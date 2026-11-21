package me.grab.hide.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;

import me.grab.hide.android.NavigationPolicy.Decision;
import org.junit.Test;

public class NavigationTest {

    @Test
    public void keepsTheAppsOwnOriginInsideAndOpensOnlyHttpsOutside() {
        assertEquals(Decision.INSIDE, NavigationPolicy.decide("https://localhost/"));
        assertEquals(Decision.INSIDE, NavigationPolicy.decide("https://localhost/p/5/?host=abc"));
        assertEquals(Decision.INSIDE, NavigationPolicy.decide("about:blank"));
        assertEquals(Decision.INSIDE, NavigationPolicy.decide("blob:https://localhost/0e3c"));
        assertEquals(Decision.OUTSIDE, NavigationPolicy.decide("https://github.com/with-hide/hide"));
        assertEquals(Decision.OUTSIDE, NavigationPolicy.decide("https://localhost:8443/"));
        assertEquals(Decision.OUTSIDE, NavigationPolicy.decide("https://localhost.evil.example/"));
    }

    @Test
    public void cancelsEveryOtherScheme() {
        String[] cancelled = {
            "http://example.com/",
            "http://localhost/",
            "tel:+15550100",
            "sms:+15550100",
            "mailto:a@example.com",
            "intent://scan/#Intent;scheme=zxing;end",
            "market://details?id=x",
            "file:///data/data/me.grab.hide.android/files/x",
            "content://me.grab.hide.android.fileprovider/x",
            "javascript:alert(1)",
            "data:text/html,hi",
            "capacitor://localhost/",
            "not a url",
            "",
        };
        for (String address : cancelled) assertEquals(address, Decision.CANCEL, NavigationPolicy.decide(address));
    }

    @Test
    public void routesABundleAddressToItsOwnPage() {
        assertEquals("/p/5/index.html", PhoneRouter.route("/p/5/"));
        assertEquals("/p/5/index.html", PhoneRouter.route("/p/5"));
        assertEquals("/p/12/index.html", PhoneRouter.route("/p/12/"));
        assertNull(PhoneRouter.route("/"));
        assertNull(PhoneRouter.route("/p/0/"));
        assertNull(PhoneRouter.route("/p/05/"));
        assertNull(PhoneRouter.route("/p/5/assets/index.js"));
        assertNull(PhoneRouter.route("/p/5/index.html"));
        assertNull(PhoneRouter.route("/p/x/"));
        assertNull(PhoneRouter.route(null));
    }
}
