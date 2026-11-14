package me.grab.hide.android;

import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * The words a decrypted notice is shown with: the twin of the iOS app's {@code NoticeWords}, one table per language the
 * app speaks, in the language of the phone's system. The message service has no web view, so it cannot read the page's
 * catalog. {@code NoticeWordsTest} holds each table equal to the iOS app's key for key, and the web side holds the iOS
 * tables equal to the catalog ({@code web/src/mobile/notificationWords.test.ts}), so a change of wording cannot reach
 * one place and not the others. A notice whose reason has a templated sentence (a plan review, a mission decision) shows
 * the plain sentence the computer sent instead ({@code detail}).
 */
public final class NoticeWords {

    public final Map<String, String> table;

    NoticeWords(Map<String, String> table) {
        this.table = table;
    }

    /** The table for the first of the phone's languages the app speaks, by the tag's first part; English otherwise. */
    public static NoticeWords forLanguages(List<String> languages) {
        for (String language : languages) {
            String code = language.toLowerCase(Locale.ROOT).split("[-_]", 2)[0];
            Map<String, String> table = TABLES.get(code);
            if (table != null) return new NoticeWords(table);
        }
        return new NoticeWords(TABLES.get("en"));
    }

    /** The word that leads the body: the row's status, else the group the notice stands in. */
    public String lead(String statusCode, String state) {
        if (statusCode != null) {
            String word = table.get("status_" + statusCode);
            if (word != null) return word;
        }
        return table.get(state);
    }

    static final Map<String, Map<String, String>> TABLES = Map.of(
        "en", Map.ofEntries(
            Map.entry("needs_you", "Needs You"), Map.entry("done", "Unread"),
            Map.entry("observer_unconfirmed", "Observer has not confirmed a warning"), Map.entry("letter_undelivered", "Letter not delivered"),
            Map.entry("status_error", "Blocked"), Map.entry("status_question", "Question"), Map.entry("status_approval", "Approval"), Map.entry("status_working", "Working"),
            Map.entry("status_done", "Done"), Map.entry("status_idle", "Idle"), Map.entry("status_unknown", "Status unknown"), Map.entry("status_waiting", "Waiting"),
            Map.entry("status_unfinished", "Unfinished"), Map.entry("status_stopped", "Stopped"), Map.entry("status_look", "To look at")
        ),
        "ko", Map.ofEntries(
            Map.entry("needs_you", "내 차례"), Map.entry("done", "읽을 것"),
            Map.entry("observer_unconfirmed", "관찰자가 경고를 확인하지 않았어요"), Map.entry("letter_undelivered", "편지가 전달되지 않았어요"),
            Map.entry("status_error", "막힘"), Map.entry("status_question", "질문"), Map.entry("status_approval", "승인"), Map.entry("status_working", "작업 중"),
            Map.entry("status_done", "끝남"), Map.entry("status_idle", "쉬는 중"), Map.entry("status_unknown", "상태 모름"), Map.entry("status_waiting", "대기"),
            Map.entry("status_unfinished", "일 남음"), Map.entry("status_stopped", "멈춤"), Map.entry("status_look", "볼 것")
        ),
        "zh", Map.ofEntries(
            Map.entry("needs_you", "轮到你"), Map.entry("done", "待读"),
            Map.entry("observer_unconfirmed", "观察者尚未确认警告"), Map.entry("letter_undelivered", "信件未送达"),
            Map.entry("status_error", "受阻"), Map.entry("status_question", "提问"), Map.entry("status_approval", "批准"), Map.entry("status_working", "工作中"),
            Map.entry("status_done", "已完成"), Map.entry("status_idle", "空闲"), Map.entry("status_unknown", "状态未知"), Map.entry("status_waiting", "等待"),
            Map.entry("status_unfinished", "未完成"), Map.entry("status_stopped", "已停止"), Map.entry("status_look", "待查看")
        ),
        "ja", Map.ofEntries(
            Map.entry("needs_you", "自分の番"), Map.entry("done", "未読"),
            Map.entry("observer_unconfirmed", "オブザーバーが警告を確認していません"), Map.entry("letter_undelivered", "レターが届いていません"),
            Map.entry("status_error", "ブロック"), Map.entry("status_question", "質問"), Map.entry("status_approval", "承認"), Map.entry("status_working", "作業中"),
            Map.entry("status_done", "完了"), Map.entry("status_idle", "アイドル"), Map.entry("status_unknown", "状態不明"), Map.entry("status_waiting", "待機"),
            Map.entry("status_unfinished", "未完了"), Map.entry("status_stopped", "停止"), Map.entry("status_look", "確認待ち")
        )
    );
}
