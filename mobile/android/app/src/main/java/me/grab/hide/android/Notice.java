package me.grab.hide.android;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.StringJoiner;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * What hided sends the phone about one agent, as {@code push::payload} builds it ({@code hided/src/mobile/push.rs}), read
 * for the parts a notification uses: the twin of the iOS app's {@code Notice}, with the same bounds, and the tags of the
 * notifications this notice closes ({@code clear}), which Android closes itself as the web app's service worker does.
 * The rest of the JSON (the subject, an older computer's {@code ask}) is not kept.
 */
public final class Notice {

    /** The largest notice the app reads: hided's own plaintext bound is 2,897 bytes ({@code push::MAX_APP_PLAINTEXT}). */
    static final int MAX_BYTES = 4096;

    public final String title;
    public final String state;
    public final String statusCode;
    public final String detail;
    public final String place;
    public final String tag;
    public final String deviceId;
    public final String paneId;
    public final String room;
    public final List<String> clear;

    private Notice(String title, String state, String statusCode, String detail, String place, String tag, String deviceId, String paneId, String room, List<String> clear) {
        this.title = title;
        this.state = state;
        this.statusCode = statusCode;
        this.detail = detail;
        this.place = place;
        this.tag = tag;
        this.deviceId = deviceId;
        this.paneId = paneId;
        this.room = room;
        this.clear = clear;
    }

    /**
     * The notice of the demo's sample question: the shape a real one has, for a computer that does not exist, so the title
     * and the body come from the same code ({@link #body}) as a real notification's.
     */
    public static Notice sample(String title, String statusCode, String state, String detail, String place) {
        return new Notice(title, state, statusCode, detail, place, "demo", "demo", "demo", "", List.of());
    }

    /** The notice in {@code json}, or null when it is not one of the shape hided sends. */
    public static Notice fromJson(byte[] json) {
        if (json.length > MAX_BYTES) return null;
        JSONObject object;
        try {
            object = new JSONObject(new String(json, StandardCharsets.UTF_8));
        } catch (JSONException e) {
            return null;
        }
        String title = text(object, "title", 1, 512);
        String state = text(object, "state", 0, 64);
        String tag = text(object, "tag", 1, 768);
        String deviceId = text(object, "device_id", 1, 256);
        String paneId = text(object, "pane_id", 1, 512);
        String room = text(object, "room", 0, 64);
        if (title == null || state == null || tag == null || deviceId == null || paneId == null || !SealedFiles.isRoom(room)) return null;
        String place = text(object, "place", 0, 512);
        return new Notice(title, state, text(object, "status_code", 0, 64), text(object, "detail", 1, 1024), place == null ? "" : place, tag, deviceId, paneId, room, tags(object.optJSONArray("clear")));
    }

    /** The body: the lead word, then the computer's sentence, then the project, as the web app's worker joins them. */
    public String body(NoticeWords words) {
        StringJoiner joined = new StringJoiner(" · ");
        for (String part : new String[] { words.lead(statusCode, state), detail, place }) {
            if (part != null && !part.isEmpty()) joined.add(part);
        }
        return joined.toString();
    }

    /** A string field of {@code min} to {@code max} characters, else null. */
    private static String text(JSONObject object, String name, int min, int max) {
        Object value = object.opt(name);
        if (!(value instanceof String)) return null;
        String text = (String) value;
        int length = text.codePointCount(0, text.length());
        return length >= min && length <= max ? text : null;
    }

    private static List<String> tags(JSONArray array) {
        if (array == null) return List.of();
        List<String> tags = new ArrayList<>();
        for (int index = 0; index < array.length(); index++) {
            Object value = array.opt(index);
            if (value instanceof String && !((String) value).isEmpty() && ((String) value).length() <= 768) tags.add((String) value);
        }
        return Collections.unmodifiableList(tags);
    }
}
