package me.grab.hide.android;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;

/**
 * Where a tap on one of the app's notifications lands: it hands the agent the notification names to {@link PushOpenInbox}
 * and brings the app's activity forward, which the page then asks for the agent ({@code HidePush.takeOpen}). It is not
 * exported, so only the app's own notifications reach it: another app cannot hand the page an agent to open, as it could
 * through the launcher activity, which is exported for the launcher and the pairing link.
 */
public final class NoticeOpenActivity extends Activity {

    static final String ROOM = "room";
    static final String DEVICE_ID = "device_id";
    static final String PANE_ID = "pane_id";

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        Intent intent = getIntent();
        // A notification of a notice the phone could not open, or the demo's sample, carries no agent: the app opens as it was.
        if (intent != null && intent.hasExtra(ROOM)) {
            PushOpenInbox.shared().offer(intent.getStringExtra(ROOM), intent.getStringExtra(DEVICE_ID), intent.getStringExtra(PANE_ID));
        }
        startActivity(new Intent(this, ShellActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        finish();
    }
}
