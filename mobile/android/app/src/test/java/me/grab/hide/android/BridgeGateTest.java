package me.grab.hide.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;

import java.util.Set;
import org.junit.Test;

public class BridgeGateTest {

    private static final Set<String> PLUGINS = BridgeGate.APP_PLUGINS;

    private static String verdict(boolean mainFrame, String origin, String type, String plugin) {
        return BridgeGate.verdict(mainFrame, origin, false, type, plugin, PLUGINS);
    }

    @Test
    public void passesAPluginCallOrAnErrorReportFromTheMainFrameOfTheAppsOrigin() {
        assertNull(verdict(true, "https://localhost", null, "HideKeychain"));
        assertNull(verdict(true, "https://localhost", null, "HideScanner"));
        assertNull(verdict(true, "https://localhost", null, "HideApp"));
        assertNull(verdict(true, "https://localhost", "js.error", null));
    }

    @Test
    public void refusesASubFrameOrAnotherOriginWhateverItAsks() {
        assertEquals("not_main_frame", verdict(false, "https://localhost", null, "HideKeychain"));
        assertEquals("not_main_frame", verdict(false, "null", null, "HideKeychain"));
        assertEquals("foreign_origin", verdict(true, "https://app.withhide.dev", null, "HideKeychain"));
        assertEquals("foreign_origin", verdict(true, "http://localhost", null, "HideKeychain"));
        assertEquals("foreign_origin", verdict(true, "https://localhost:8443", null, "HideKeychain"));
    }

    @Test
    public void refusesCapacitorsOwnPluginsAPluginItDoesNotNameAndCordova() {
        for (String plugin : new String[] { "WebView", "CapacitorHttp", "CapacitorCookies", "SystemBars", "HidePush", "Console", "hideapp", "" }) {
            assertEquals(plugin, "plugin_not_exposed", verdict(true, "https://localhost", null, plugin));
        }
        assertEquals("plugin_not_exposed", verdict(true, "https://localhost", null, null));
        assertEquals("call_type", verdict(true, "https://localhost", "cordova", "HideApp"));
        assertEquals("call_type", verdict(true, "https://localhost", "message", "HideApp"));
        assertEquals("malformed", BridgeGate.verdict(true, "https://localhost", true, null, null, PLUGINS));
    }

    @Test
    public void namesTheAppsThreePluginsAndNoOther() {
        assertEquals(Set.of("HideKeychain", "HideScanner", "HideApp"), BridgeGate.APP_PLUGINS);
    }
}
