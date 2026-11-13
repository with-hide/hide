package me.grab.hide.android;

import android.util.Log;
import com.google.firebase.messaging.FirebaseMessagingService;
import com.google.firebase.messaging.RemoteMessage;
import java.util.List;

/**
 * Receives the relay's FCM data message, the twin of the iOS app's notification extension: it reads the computers' keys
 * ({@link PushKeyStore}), decrypts the notice ({@link NoticeOpener}) and shows it ({@link Notices}). In every other case it
 * shows the fixed text, never nothing: Google lowers the priority of an app whose high-priority messages show nothing.
 * Nothing it decrypts is logged; a log line names what happened, never a title, an agent or a room.
 */
public class HidePushService extends FirebaseMessagingService {

    private static final String TAG = "HidePush";

    @Override
    public void onMessageReceived(RemoteMessage message) {
        String hide = message.getData().get("hide");
        Notice notice = null;
        if (hide != null) {
            try {
                List<PushKeyStore.Key> keys = PushKeyStore.of(this).all();
                notice = NoticeOpener.open(hide, keys);
            } catch (SealedFiles.Failure failure) {
                Log.w(TAG, "notice keys unreadable: " + failure.code);
            }
        }
        if (notice != null) {
            Notices.show(this, notice);
        } else {
            Log.i(TAG, hide == null ? "message without a notice" : "notice not opened");
            Notices.showFixed(this);
        }
    }

    /**
     * Nothing is kept: every connection to a computer sends it this phone's registration again with the token Firebase
     * holds then ({@code syncAppPush} on the page), the way the iOS app handles a changed device token.
     */
    @Override
    public void onNewToken(String token) {}
}
