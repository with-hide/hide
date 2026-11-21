package me.grab.hide.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;

import java.nio.charset.StandardCharsets;
import java.util.List;
import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.Test;

/** What the message service makes of a body: which key opens it, what it reads, and the words it shows. */
public class NoticeTest {

    private static final String ROOM = "A".repeat(43);
    private static final String OTHER = "B".repeat(43);

    private static PushKeyStore.Key key(String room) {
        PushCipher.Pair pair = PushCipher.generate();
        byte[] auth = new byte[16];
        for (int index = 0; index < 16; index++) auth[index] = (byte) index;
        return new PushKeyStore.Key(room, pair.privateKey, pair.publicKey, auth);
    }

    private static String seal(JSONObject notice, PushKeyStore.Key key) throws Exception {
        return NoticeSealer.seal(notice.toString().getBytes(StandardCharsets.UTF_8), key.publicKey, key.auth);
    }

    private static Notice read(JSONObject notice) {
        return Notice.fromJson(notice.toString().getBytes(StandardCharsets.UTF_8));
    }

    @Test
    public void opensTheNoticeWithTheKeyOfItsComputerAmongSeveral() throws Exception {
        PushKeyStore.Key mine = key(ROOM), theirs = key(OTHER);
        Notice opened = NoticeOpener.open(seal(NoticeSealer.notice(ROOM), mine), List.of(theirs, mine));
        assertNotNull(opened);
        assertEquals("Fix the build", opened.title);
        assertEquals("dev|w1:p2", opened.tag);
        assertEquals(List.of(ROOM, "dev", "w1:p2"), List.of(opened.room, opened.deviceId, opened.paneId));
        assertEquals(List.of("dev|w1:p1"), opened.clear);
    }

    @Test
    public void staysClosedWithNoKeyWithAnotherComputersKeyAndForARoomTheKeyIsNot() throws Exception {
        PushKeyStore.Key mine = key(ROOM);
        String body = seal(NoticeSealer.notice(ROOM), mine);
        assertNull("a computer that was removed has no key", NoticeOpener.open(body, List.of()));
        assertNull(NoticeOpener.open(body, List.of(key(OTHER))));
        assertNull(NoticeOpener.open("not base64 !", List.of(mine)));
        assertNull("this key's computer did not send a notice naming another", NoticeOpener.open(seal(NoticeSealer.notice(OTHER), mine), List.of(mine)));
    }

    @Test
    public void readsNoOddlyShapedNoticeAndIgnoresWhatItDoesNotUse() throws Exception {
        assertNull(read(NoticeSealer.notice(ROOM).put("title", "")));
        assertNull(read(NoticeSealer.notice(ROOM).put("room", "short")));
        assertNull(read(NoticeSealer.notice(ROOM).put("tag", JSONObject.NULL)));
        assertNull(read(NoticeSealer.notice(ROOM).put("title", "x".repeat(513))));
        assertNull(Notice.fromJson(" ".repeat(5000).getBytes(StandardCharsets.UTF_8)));
        assertNull(Notice.fromJson("[]".getBytes(StandardCharsets.UTF_8)));
        // An older computer's ask, a missing status and an odd clear list do not stop a notice.
        Notice older = read(NoticeSealer.notice(ROOM).put("ask", new JSONObject().put("kind", "question").put("turn", "12ab"))
            .put("status_code", JSONObject.NULL).put("detail", "").put("place", JSONObject.NULL).put("clear", new JSONArray().put(3).put("").put("a|b")));
        assertNotNull(older);
        assertNull(older.statusCode);
        assertNull(older.detail);
        assertEquals("", older.place);
        assertEquals(List.of("a|b"), older.clear);
        assertEquals(List.of(), read(NoticeSealer.notice(ROOM).put("clear", JSONObject.NULL)).clear);
    }

    @Test
    public void wordsTheBodyInThePhonesLanguageAndFallsBackToEnglish() throws Exception {
        Notice asking = read(NoticeSealer.notice(ROOM).put("detail", "Which branch?"));
        assertEquals("질문 · Which branch? · contong", asking.body(NoticeWords.forLanguages(List.of("ko-KR"))));
        assertEquals("Question · Which branch? · contong", asking.body(NoticeWords.forLanguages(List.of("en-US"))));
        assertEquals("질문 · Which branch? · contong", asking.body(NoticeWords.forLanguages(List.of("fr-FR", "ko"))));
        assertEquals("Question · Which branch? · contong", asking.body(NoticeWords.forLanguages(List.of("fr-FR"))));
        assertEquals("提问 · Which branch? · contong", asking.body(NoticeWords.forLanguages(List.of("zh-Hans-CN"))));
        Notice letter = read(NoticeSealer.notice(ROOM).put("state", "letter_undelivered").put("status_code", JSONObject.NULL).put("detail", JSONObject.NULL).put("place", "letter-7"));
        assertEquals("Letter not delivered · letter-7", letter.body(NoticeWords.forLanguages(List.of("en"))));
    }

    @Test
    public void theDemoSampleIsWordedAsARealQuestionIs() {
        Notice sample = Notice.sample("Address review comments", "question", "needs_you", "Apply all 3 review comments?", "demo · fix/review");
        assertEquals("Address review comments", sample.title);
        assertEquals("Question · Apply all 3 review comments? · demo · fix/review", sample.body(NoticeWords.forLanguages(List.of("en"))));
    }

    @Test
    public void aTapOpensTheAgentOnlyForTheShapesTheServiceWritesAndOnce() {
        PushOpenInbox inbox = new PushOpenInbox();
        int[] told = { 0 };
        inbox.setListener(() -> told[0]++);
        assertEquals(false, inbox.offer(null, null, null));
        assertEquals(false, inbox.offer("short", "mini", "w1:p1"));
        assertEquals(false, inbox.offer(ROOM, "", "w1:p1"));
        assertEquals(false, inbox.offer(ROOM, "mini", "x".repeat(513)));
        assertNull(inbox.take());
        assertEquals(true, inbox.offer(ROOM, "mini", "w1:p1"));
        assertEquals(1, told[0]);
        PushOpenInbox.Target target = inbox.take();
        assertEquals(List.of(ROOM, "mini", "w1:p1"), List.of(target.room, target.deviceId, target.paneId));
        assertNull("given out once", inbox.take());
    }
}
