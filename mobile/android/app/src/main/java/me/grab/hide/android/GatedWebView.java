package me.grab.hide.android;

import android.annotation.SuppressLint;
import android.content.Context;
import android.util.AttributeSet;
import android.util.Log;
import com.getcapacitor.CapacitorWebView;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

/**
 * The app's web view, Capacitor's with one change: it takes no Java object into a page. A Java object answers every frame,
 * the sandboxed one that draws agent-made HTML among them. Capacitor 8.6 puts one in for its bridge when it cannot add its
 * web message listener ({@code MessageHandler}), and two of its plugins put one in each; refused here, none is ever in a
 * page. The web view keeps the names it refused, so {@link ShellActivity} can tell that Capacitor fell back to its Java
 * object bridge and make no bridge at all.
 *
 * <p>{@code res/layout/capacitor_bridge_layout_main.xml} puts it in place of Capacitor's: the app's layout of that name
 * replaces the library's.
 */
public class GatedWebView extends CapacitorWebView {

    private static final String TAG = "HideBridge";

    private final Set<String> refused = ConcurrentHashMap.newKeySet();

    public GatedWebView(Context context, AttributeSet attrs) {
        super(context, attrs);
    }

    @SuppressLint("JavascriptInterface")
    @Override
    public void addJavascriptInterface(Object object, String name) {
        refused.add(name);
        Log.w(TAG, "java object refused: " + name);
    }

    /** True once something tried to put a Java object of this name into the page. */
    boolean refused(String name) {
        return refused.contains(name);
    }
}
