package me.grab.hide.android;

import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;

/**
 * What the web app asks of the app itself, the same calls as the iOS app's {@code HideApp}: its version, the pairing
 * link that opened it, and the light or dark the page shows (which the system bars follow).
 */
@CapacitorPlugin(name = "HideApp")
public class HideAppPlugin extends Plugin {

    private final PairLinkInbox inbox = PairLinkInbox.shared();

    @Override
    public void load() {
        // The event carries no link: the page asks for it with `takeLink`, so a secret never travels in an event.
        inbox.setListener(() -> notifyListeners("link", new JSObject()));
    }

    @Override
    protected void handleOnDestroy() {
        inbox.setListener(null);
    }

    @PluginMethod
    public void info(PluginCall call) {
        JSObject answer = new JSObject();
        try {
            PackageInfo info = getContext().getPackageManager().getPackageInfo(getContext().getPackageName(), 0);
            answer.put("version", info.versionName == null ? "" : info.versionName);
            answer.put("build", String.valueOf(info.getLongVersionCode()));
        } catch (PackageManager.NameNotFoundException e) {
            answer.put("version", "");
            answer.put("build", "");
        }
        call.resolve(answer);
    }

    @PluginMethod
    public void takeLink(PluginCall call) {
        JSObject answer = new JSObject();
        String link = inbox.take();
        if (link != null) answer.put("link", link);
        call.resolve(answer);
    }

    @PluginMethod
    public void appearance(PluginCall call) {
        String scheme = call.getString("scheme", "");
        if (!scheme.equals("dark") && !scheme.equals("light") && !scheme.equals("system")) {
            call.reject("invalid_scheme", "invalid_scheme");
            return;
        }
        getActivity().runOnUiThread(() -> {
            ((ShellActivity) getActivity()).setAppearance(scheme);
            call.resolve();
        });
    }
}
