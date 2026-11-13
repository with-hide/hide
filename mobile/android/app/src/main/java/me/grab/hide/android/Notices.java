package me.grab.hide.android;

import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.LocaleList;
import androidx.core.app.NotificationCompat;
import androidx.core.app.NotificationManagerCompat;
import java.util.ArrayList;
import java.util.List;

/**
 * The app's notifications on the phone: a decrypted notice, the fixed text of one the phone could not open, and the demo's
 * sample. One notification per agent: a newer notice replaces the one with the same tag, and the tags a notice names to
 * close ({@code clear}) are closed when it arrives, as the web app's service worker does. A tap goes to
 * {@link NoticeOpenActivity} with the agent the notice named, or with none.
 */
final class Notices {

    static final String CHANNEL = "notices";
    /** Every notification of the app has this id; its tag tells them apart. */
    static final int ID = 1;
    /** The fixed text of notices the phone could not open: one notification, however many arrive. */
    static final String FIXED_TAG = "hide.fixed";
    static final String DEMO_TAG = "hide.demo";

    private Notices() {}

    /** Whether the system shows the app's notifications now: on Android 13 and later, also whether the person allowed them. */
    static boolean allowed(Context context) {
        return NotificationManagerCompat.from(context).areNotificationsEnabled();
    }

    /** The words of the phone's system languages, as the iOS extension reads them: the page's own choice is not readable here. */
    static NoticeWords words() {
        LocaleList locales = LocaleList.getDefault();
        List<String> languages = new ArrayList<>();
        for (int index = 0; index < locales.size(); index++) languages.add(locales.get(index).toLanguageTag());
        return NoticeWords.forLanguages(languages);
    }

    static void show(Context context, Notice notice) {
        NotificationManager manager = manager(context);
        for (String tag : notice.clear) manager.cancel(tag, ID);
        Intent open = new Intent(context, NoticeOpenActivity.class)
            // One pending intent per agent: extras alone do not tell two pending intents apart.
            .setData(Uri.fromParts("hide-notice", notice.tag, null))
            .putExtra(NoticeOpenActivity.ROOM, notice.room)
            .putExtra(NoticeOpenActivity.DEVICE_ID, notice.deviceId)
            .putExtra(NoticeOpenActivity.PANE_ID, notice.paneId);
        manager.notify(notice.tag, ID, build(context, notice.title, notice.body(words()), open));
    }

    static void showFixed(Context context) {
        String text = context.getString(R.string.notice_fixed);
        manager(context).notify(FIXED_TAG, ID, build(context, context.getString(R.string.app_name), text, new Intent(context, NoticeOpenActivity.class)));
    }

    static void showSample(Context context, Notice sample) {
        manager(context).notify(DEMO_TAG, ID, build(context, sample.title, sample.body(words()), new Intent(context, NoticeOpenActivity.class)));
    }

    private static NotificationManager manager(Context context) {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager.getNotificationChannel(CHANNEL) == null) {
            manager.createNotificationChannel(new NotificationChannel(CHANNEL, context.getString(R.string.notice_channel), NotificationManager.IMPORTANCE_HIGH));
        }
        return manager;
    }

    private static android.app.Notification build(Context context, String title, String text, Intent open) {
        PendingIntent tap = PendingIntent.getActivity(context, 0, open, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        return new NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(new NotificationCompat.BigTextStyle().bigText(text))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setContentIntent(tap)
            .setAutoCancel(true)
            .build();
    }
}
