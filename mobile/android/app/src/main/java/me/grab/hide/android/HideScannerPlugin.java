package me.grab.hide.android;

import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import com.google.mlkit.vision.barcode.common.Barcode;
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions;
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning;

/**
 * The scanner behind the QR scan button, the same call as the iOS app's {@code HideScanner}: the text of one Hide
 * pairing link ({@link PairLink}), which the web app decodes with the decoder every pairing uses.
 *
 * <p>Google's code scanner draws the camera screen in Google Play services and gives the app only the code it read, so
 * the app asks for no camera permission and draws no screen of its own; the words the page sends ({@code text}, which
 * the iOS scanner draws) are checked as iOS checks them and not shown. The scanner closes on the first code it reads: a
 * code that is not a pairing link opens it again, as the iOS scanner keeps looking. Closing it answers {@code cancelled};
 * a scanner Play services cannot give answers {@code unavailable}.
 */
@CapacitorPlugin(name = "HideScanner")
public class HideScannerPlugin extends Plugin {

    static final String[] TEXT_KEYS = { "title", "hint", "whereTitle", "whereBody", "deniedTitle", "deniedBody", "openSettings", "close" };

    private boolean scanning;

    /** True when every word the iOS scanner draws is there, as a non-empty string. */
    static boolean validText(JSObject text) {
        if (text == null) return false;
        for (String key : TEXT_KEYS) {
            Object value = text.opt(key);
            if (!(value instanceof String) || ((String) value).isEmpty()) return false;
        }
        return true;
    }

    @PluginMethod
    public void scan(PluginCall call) {
        if (!validText(call.getObject("text"))) {
            call.reject("invalid_text", "invalid_text");
            return;
        }
        getActivity().runOnUiThread(() -> {
            if (scanning) {
                call.reject("busy", "busy");
                return;
            }
            scanning = true;
            read(call);
        });
    }

    private void read(PluginCall call) {
        GmsBarcodeScannerOptions options = new GmsBarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build();
        // Not tied to the activity: the scanner's own screen stops it, and a listener bound to it would be dropped then.
        // Task listeners run on the main thread, where `scanning` is read and written.
        GmsBarcodeScanning.getClient(getActivity(), options)
            .startScan()
            .addOnSuccessListener((barcode) -> {
                String link = PairLink.accept(barcode.getRawValue());
                if (link == null) {
                    read(call);
                    return;
                }
                scanning = false;
                JSObject answer = new JSObject();
                answer.put("value", link);
                call.resolve(answer);
            })
            .addOnCanceledListener(() -> {
                scanning = false;
                call.reject("cancelled", "cancelled");
            })
            .addOnFailureListener((error) -> {
                scanning = false;
                call.reject("unavailable", "unavailable");
            });
    }
}
