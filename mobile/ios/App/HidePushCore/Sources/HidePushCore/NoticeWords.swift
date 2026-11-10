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

    /// What a failed Reply says, in the same language. These four sentences exist only here (the web app never sends a
    /// Reply from a notification), so they are in the table beside the status words but outside the catalog drift test.
    public var reply: ReplyWords {
        ReplyWords(stale: table["reply_stale"] ?? "", notSent: table["reply_not_sent"] ?? "",
                   outdated: table["reply_outdated"] ?? "", unconfirmed: table["reply_unconfirmed"] ?? "")
    }

    static let tables: [String: [String: String]] = [
        "en": [
            "needs_you": "Needs You", "done": "Unread",
            "observer_unconfirmed": "Observer has not confirmed a warning", "letter_undelivered": "Letter not delivered",
            "status_error": "Blocked", "status_question": "Question", "status_approval": "Approval", "status_working": "Working",
            "status_done": "Done", "status_idle": "Idle", "status_unknown": "Status unknown", "status_waiting": "Waiting",
            "status_unfinished": "Unfinished", "status_stopped": "Stopped", "status_look": "To look at",
            "reply_stale": "That question is no longer waiting. Your reply was not sent. Tap to see it.",
            "reply_not_sent": "Your reply was not sent. Tap to open Hide and try again.",
            "reply_outdated": "Update Hide on the computer to reply from a notification.",
            "reply_unconfirmed": "Could not confirm your reply reached the computer. Tap to check.",
        ],
        "ko": [
            "needs_you": "내 차례", "done": "읽을 것",
            "observer_unconfirmed": "관찰자가 경고를 확인하지 않았어요", "letter_undelivered": "편지가 전달되지 않았어요",
            "status_error": "막힘", "status_question": "질문", "status_approval": "승인", "status_working": "작업 중",
            "status_done": "끝남", "status_idle": "쉬는 중", "status_unknown": "상태 모름", "status_waiting": "대기",
            "status_unfinished": "일 남음", "status_stopped": "멈춤", "status_look": "볼 것",
            "reply_stale": "그 질문은 더 이상 기다리지 않아요. 답장을 보내지 못했어요. 눌러서 확인하세요.",
            "reply_not_sent": "답장을 보내지 못했어요. 눌러서 Hide를 열고 다시 시도하세요.",
            "reply_outdated": "알림에서 답장하려면 컴퓨터의 Hide를 업데이트하세요.",
            "reply_unconfirmed": "답장이 컴퓨터에 닿았는지 확인하지 못했어요. 눌러서 확인하세요.",
        ],
        "zh": [
            "needs_you": "轮到你", "done": "待读",
            "observer_unconfirmed": "观察者尚未确认警告", "letter_undelivered": "信件未送达",
            "status_error": "受阻", "status_question": "提问", "status_approval": "批准", "status_working": "工作中",
            "status_done": "已完成", "status_idle": "空闲", "status_unknown": "状态未知", "status_waiting": "等待",
            "status_unfinished": "未完成", "status_stopped": "已停止", "status_look": "待查看",
            "reply_stale": "该提问已不再等待。你的回复未发送。点按查看。",
            "reply_not_sent": "你的回复未发送。点按打开 Hide 后重试。",
            "reply_outdated": "请更新电脑上的 Hide，以便从通知中回复。",
            "reply_unconfirmed": "无法确认你的回复已到达电脑。点按查看。",
        ],
        "ja": [
            "needs_you": "自分の番", "done": "未読",
            "observer_unconfirmed": "オブザーバーが警告を確認していません", "letter_undelivered": "レターが届いていません",
            "status_error": "ブロック", "status_question": "質問", "status_approval": "承認", "status_working": "作業中",
            "status_done": "完了", "status_idle": "アイドル", "status_unknown": "状態不明", "status_waiting": "待機",
            "status_unfinished": "未完了", "status_stopped": "停止", "status_look": "確認待ち",
            "reply_stale": "その質問はもう待っていません。返信は送信されませんでした。タップして確認してください。",
            "reply_not_sent": "返信は送信されませんでした。タップしてHideを開き、もう一度お試しください。",
            "reply_outdated": "通知から返信するには、コンピュータのHideを更新してください。",
            "reply_unconfirmed": "返信がコンピュータに届いたか確認できませんでした。タップして確認してください。",
        ],
    ]
}
