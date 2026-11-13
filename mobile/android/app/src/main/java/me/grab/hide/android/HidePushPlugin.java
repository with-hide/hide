package me.grab.hide.android;

import android.Manifest;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.util.Log;
import com.getcapacitor.JSObject;
import com.getcapacitor.PermissionState;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import com.getcapacitor.annotation.Permission;
import com.getcapacitor.annotation.PermissionCallback;
import com.google.android.gms.tasks.Tasks;
import com.google.firebase.messaging.FirebaseMessaging;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.Executor;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * The web app's way to the app's notifications, the same calls and answers as the iOS app's {@code HidePush}: whether they
 * are allowed, turning them on for one computer, the registration to send it again, removing a computer's key, the agent a
 * tap named, the demo's sample and the system's notification settings. There is no call that reads a private key, sends
 * an action or takes a key of the caller's own, and a frame that is not the app's main frame never reaches this class
 * ({@link BridgeGate}). The answer to {@code enable} and {@code registration} names Google's service ({@code service:
 * "fcm"}) and has no environment, since FCM has one host.
 */
@CapacitorPlugin(name = "HidePush", permissions = @Permission(strings = { Manifest.permission.POST_NOTIFICATIONS }, alias = HidePushPlugin.NOTIFICATIONS))
public class HidePushPlugin extends Plugin {

    static final String NOTIFICATIONS = "notifications";
    private static final String TAG = "HidePush";
    /** How long a request waits for Firebase's token before it answers {@code token_timeout}, as the iOS app waits for Apple's. */
    private static final long TOKEN_WAIT_MS = 10_000;

    private final PushOpenInbox inbox = PushOpenInbox.shared();
    private final Handler main = new Handler(Looper.getMainLooper());
    private PushKeyStore store;
    /** Keystore work and Firebase's answers run here, off the main thread and off the plugin thread that takes the next call. */
    private ExecutorService work;
    /**
     * Where Firebase's answers go: {@link #work}, or nowhere once the plugin has ended, since Firebase answers on a thread of
     * its own after the activity may be gone and a call of a destroyed plugin has no page to answer.
     */
    private final Executor answers = (task) -> {
        try {
            work.execute(task);
        } catch (RejectedExecutionException ended) {
            Log.i(TAG, "an answer came after the plugin ended");
        }
    };

    @Override
    public void load() {
        store = PushKeyStore.of(getContext());
        work = Executors.newSingleThreadExecutor();
        // The event carries no target: the page asks for it with `takeOpen`.
        inbox.setListener(() -> notifyListeners("open", new JSObject()));
    }

    @Override
    protected void handleOnDestroy() {
        inbox.setListener(null);
        main.removeCallbacksAndMessages(null);
        work.shutdownNow();
    }

    private static void fail(PluginCall call, String code) {
        call.reject(code, code);
    }

    private static String room(PluginCall call) {
        String room = call.getString("room");
        if (SealedFiles.isRoom(room)) return room;
        fail(call, "invalid_room");
        return null;
    }

    /**
     * {@code unknown} while the system may still ask the person, {@code denied} when it will not (a refusal it no longer
     * asks about, or the app's notifications turned off in the system's settings), {@code granted} otherwise. Before
     * Android 13 there is no question: notifications are on until the person turns them off.
     */
    String permission() {
        if (Notices.allowed(getContext())) return "granted";
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return "denied";
        PermissionState state = getPermissionState(NOTIFICATIONS);
        return state == PermissionState.PROMPT || state == PermissionState.PROMPT_WITH_RATIONALE ? "unknown" : "denied";
    }

    @PluginMethod
    public void state(PluginCall call) {
        JSObject answer = new JSObject();
        answer.put("permission", permission());
        call.resolve(answer);
    }

    /**
     * Turns notifications on for one computer: asks the system only now (this is called from the person's tap), gets
     * Firebase's token, makes the computer's key and answers with what the computer needs to encrypt for it.
     */
    @PluginMethod
    public void enable(PluginCall call) {
        if (room(call) == null) return;
        if (permission().equals("unknown")) {
            requestPermissionForAlias(NOTIFICATIONS, call, "enableAsked");
            return;
        }
        enableAsked(call);
    }

    @PermissionCallback
    private void enableAsked(PluginCall call) {
        if (!Notices.allowed(getContext())) {
            fail(call, "denied");
            return;
        }
        // Firebase keeps its token current from now on, as a registration with Apple does; `forget` of the last key ends it.
        FirebaseMessaging.getInstance().setAutoInitEnabled(true);
        registration(call, call.getString("room"), true);
    }

    /**
     * The registration to send again on a connection: only for a computer whose key exists and only while the system still
     * shows notifications. Nothing is created here: the key is read before Firebase is asked, so a computer with no key
     * makes no Firebase installation or token, as on iOS.
     */
    @PluginMethod
    public void registration(PluginCall call) {
        String room = room(call);
        if (room == null) return;
        if (!Notices.allowed(getContext())) {
            call.resolve(notFound());
            return;
        }
        work.execute(() -> {
            try {
                if (store.read(room) == null) {
                    call.resolve(notFound());
                    return;
                }
            } catch (SealedFiles.Failure failure) {
                fail(call, failure.code);
                return;
            }
            registration(call, room, false);
        });
    }

    private void registration(PluginCall call, String room, boolean create) {
        AtomicBoolean answered = new AtomicBoolean(false);
        main.postDelayed(() -> {
            if (answered.compareAndSet(false, true)) fail(call, "token_timeout");
        }, TOKEN_WAIT_MS);
        FirebaseMessaging.getInstance().getToken().addOnCompleteListener(answers, (task) -> {
            if (!answered.compareAndSet(false, true)) return;
            if (!task.isSuccessful() || task.getResult() == null) {
                // Google's reason (no Play services, no network) is the system's; the page gets the one word.
                Log.w(TAG, "no notification token: " + (task.getException() == null ? "empty" : task.getException().getClass().getSimpleName()));
                fail(call, "token_unavailable");
                return;
            }
            try {
                PushKeyStore.Key key = create ? store.readOrCreate(room) : store.read(room);
                if (key == null) {
                    call.resolve(notFound());
                    return;
                }
                JSObject answer = new JSObject();
                answer.put("found", true);
                answer.put("service", "fcm");
                answer.put("token", task.getResult());
                answer.put("p256dh", PushCipher.toBase64Url(key.publicKey));
                answer.put("auth", PushCipher.toBase64Url(key.auth));
                call.resolve(answer);
            } catch (SealedFiles.Failure failure) {
                fail(call, failure.code);
            }
        });
    }

    private static JSObject notFound() {
        JSObject answer = new JSObject();
        answer.put("found", false);
        return answer;
    }

    /**
     * Deletes a computer's key: its notices open no more. With none left no notice could be opened, so the app gives its
     * token back and Google answers {@code UNREGISTERED} to every computer that still sends one, including one removed
     * while it was off and could not be told, as the iOS app unregisters from Apple. It answers once the token is deleted,
     * so an {@code enable} right after it gets a new token, never the one being deleted.
     */
    @PluginMethod
    public void forget(PluginCall call) {
        String room = room(call);
        if (room == null) return;
        work.execute(() -> {
            try {
                store.remove(room);
            } catch (SealedFiles.Failure failure) {
                fail(call, failure.code);
                return;
            }
            try {
                // A key list that cannot be read is not "none left": nothing is given back on a guess.
                if (store.rooms().isEmpty()) {
                    FirebaseMessaging.getInstance().setAutoInitEnabled(false);
                    Tasks.await(FirebaseMessaging.getInstance().deleteToken(), TOKEN_WAIT_MS, TimeUnit.MILLISECONDS);
                }
            } catch (SealedFiles.Failure failure) {
                Log.w(TAG, "notice keys unreadable after a removal: " + failure.code);
            } catch (ExecutionException | TimeoutException e) {
                // The key is gone, so no notice opens; a token Google keeps answers with the fixed text until it expires.
                Log.w(TAG, "notification token not deleted: " + e.getClass().getSimpleName());
            } catch (InterruptedException e) {
                // The plugin is ending (shutdownNow): its call ends with it.
                Thread.currentThread().interrupt();
                return;
            }
            call.resolve();
        });
    }

    @PluginMethod
    public void takeOpen(PluginCall call) {
        PushOpenInbox.Target target = inbox.take();
        JSObject answer = new JSObject();
        if (target != null) {
            answer.put("room", target.room);
            answer.put("device_id", target.deviceId);
            answer.put("pane_id", target.paneId);
        }
        call.resolve(answer);
    }

    /**
     * A local notification for the demo mode: no token, no key, no network. It is worded by the same {@link Notice} code as
     * a real question (the agent's title, then the status word, the sentence and the project), and a tap opens the app.
     */
    @PluginMethod
    public void demoSample(PluginCall call) {
        if (permission().equals("unknown")) {
            requestPermissionForAlias(NOTIFICATIONS, call, "demoAsked");
            return;
        }
        demoAsked(call);
    }

    @PermissionCallback
    private void demoAsked(PluginCall call) {
        if (!Notices.allowed(getContext())) {
            fail(call, "denied");
            return;
        }
        JSObject words = call.getObject("words", new JSObject());
        Notices.showSample(getContext(), Notice.sample(word(words, "title"), word(words, "status_code"), word(words, "state"), word(words, "detail"), word(words, "place")));
        call.resolve();
    }

    private static String word(JSObject words, String key) {
        String value = words.optString(key, "");
        return value.length() > 200 ? value.substring(0, 200) : value;
    }

    /** This app's notification settings, where a refused permission is turned on again. No address comes from the caller. */
    @PluginMethod
    public void openSettings(PluginCall call) {
        Intent settings = new Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
            .putExtra(Settings.EXTRA_APP_PACKAGE, getContext().getPackageName())
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        try {
            getContext().startActivity(settings);
            call.resolve();
        } catch (ActivityNotFoundException e) {
            fail(call, "settings_unavailable");
        }
    }
}
