import Foundation

/// The words a decrypted notice is shown with. The phone's web app words these from its catalog and hands them to
/// its service worker; the notification extension has no web view, so the same short list lives here, one table per
/// language the app speaks. `web/src/mobile/notificationWords.test.ts` holds each entry equal to the catalog's, so a
/// change of wording cannot reach one place and not the other.
/// A notice whose reason has a templated sentence (a plan review, a mission decision) shows the plain sentence the
/// computer sent instead (`detail`), which is what the service worker does for a cause it has no words for.
public struct NoticeWords: Equatable {
    public let table: [String: String]

    public init(table: [String: String]) { self.table = table }

    /// The table for the first of the phone's preferred languages the app speaks; English otherwise.
    public static func forLanguages(_ languages: [String]) -> NoticeWords {
        for language in languages {
            let code = language.lowercased().split(whereSeparator: { $0 == "-" || $0 == "_" }).first.map(String.init) ?? ""
            if let table = tables[code] { return NoticeWords(table: table) }
        }
        return NoticeWords(table: tables["en"]!)
    }

    public static var current: NoticeWords { forLanguages(Locale.preferredLanguages) }

    /// The word that leads the body: the row's status, else the group the notice stands in.
    public func lead(statusCode: String?, state: String) -> String? {
        if let statusCode, let word = table["status_\(statusCode)"] { return word }
        return table[state]
    }

    static let tables: [String: [String: String]] = [
        "en": [
            "needs_you": "Needs You", "done": "Unread",
            "observer_unconfirmed": "Observer has not confirmed a warning", "letter_undelivered": "Letter not delivered",
            "status_error": "Blocked", "status_question": "Question", "status_approval": "Approval", "status_working": "Working",
            "status_done": "Done", "status_idle": "Idle", "status_unknown": "Status unknown", "status_waiting": "Waiting",
            "status_unfinished": "Unfinished", "status_stopped": "Stopped", "status_look": "To look at",
        ],
        "ko": [
            "needs_you": "내 차례", "done": "읽을 것",
            "observer_unconfirmed": "관찰자가 경고를 확인하지 않았어요", "letter_undelivered": "편지가 전달되지 않았어요",
            "status_error": "막힘", "status_question": "질문", "status_approval": "승인", "status_working": "작업 중",
            "status_done": "끝남", "status_idle": "쉬는 중", "status_unknown": "상태 모름", "status_waiting": "대기",
            "status_unfinished": "일 남음", "status_stopped": "멈춤", "status_look": "볼 것",
        ],
        "zh": [
            "needs_you": "轮到你", "done": "待读",
            "observer_unconfirmed": "观察者尚未确认警告", "letter_undelivered": "信件未送达",
            "status_error": "受阻", "status_question": "提问", "status_approval": "批准", "status_working": "工作中",
            "status_done": "已完成", "status_idle": "空闲", "status_unknown": "状态未知", "status_waiting": "等待",
            "status_unfinished": "未完成", "status_stopped": "已停止", "status_look": "待查看",
        ],
        "ja": [
            "needs_you": "自分の番", "done": "未読",
            "observer_unconfirmed": "オブザーバーが警告を確認していません", "letter_undelivered": "レターが届いていません",
            "status_error": "ブロック", "status_question": "質問", "status_approval": "承認", "status_working": "作業中",
            "status_done": "完了", "status_idle": "アイドル", "status_unknown": "状態不明", "status_waiting": "待機",
            "status_unfinished": "未完了", "status_stopped": "停止", "status_look": "確認待ち",
        ],
    ]
}
