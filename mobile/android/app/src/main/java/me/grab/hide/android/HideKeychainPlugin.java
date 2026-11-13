package me.grab.hide.android;

import com.getcapacitor.JSArray;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;

/**
 * The web app's way to the pairing store, the same calls and answers as the iOS app's {@code HideKeychain}: save, read,
 * remove and list a computer's item by its room, and nothing else. There is no call that takes a key of the caller's
 * own, and a frame that is not the app's main frame never reaches this class ({@link BridgeGate}).
 */
@CapacitorPlugin(name = "HideKeychain")
public class HideKeychainPlugin extends Plugin {

    private PairingStore store;

    @Override
    public void load() {
        store = PairingStore.of(getContext());
    }

    private static void fail(PluginCall call, PairingStore.Failure failure) {
        call.reject(failure.code, failure.code);
    }

    @PluginMethod
    public void save(PluginCall call) {
        try {
            store.save(new PairingStore.Item(call.getString("room", ""), call.getString("noiseKey", ""), call.getString("credential")));
            call.resolve();
        } catch (PairingStore.Failure failure) {
            fail(call, failure);
        }
    }

    @PluginMethod
    public void read(PluginCall call) {
        try {
            PairingStore.Item item = store.read(call.getString("room", ""));
            JSObject answer = new JSObject();
            answer.put("found", item != null);
            if (item != null) {
                answer.put("noiseKey", item.noiseKey);
                if (item.credential != null) answer.put("credential", item.credential);
            }
            call.resolve(answer);
        } catch (PairingStore.Failure failure) {
            fail(call, failure);
        }
    }

    @PluginMethod
    public void remove(PluginCall call) {
        try {
            store.remove(call.getString("room", ""));
            call.resolve();
        } catch (PairingStore.Failure failure) {
            fail(call, failure);
        }
    }

    @PluginMethod
    public void list(PluginCall call) {
        try {
            JSObject answer = new JSObject();
            answer.put("rooms", new JSArray(store.list()));
            call.resolve(answer);
        } catch (PairingStore.Failure failure) {
            fail(call, failure);
        }
    }
}
