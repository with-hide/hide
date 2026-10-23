// The words for a GitHub read that failed where the core sends only a code:
// a checkout card's `github.failure_category`, a PR's checks and review
// comments (`pr_work.feedback.failure`) and an issue's body
// (`issue_work.detail.failure`); the name of a failed check GitHub gives none
// (`failed_checks[].name` null); and where to run the sign-in command. `gh`'s
// own words stay in the core's diagnostics (docs/LOCALIZATION.md, Translation
// boundary).
import type { Catalogs } from "../schema";

export const githubReasonsEnglish = {
  "githubReason.not_installed": "gh isn't installed",
  "githubReason.not_logged_in": "Not signed in to gh",
  "githubReason.network_or_rate_limit": "GitHub didn't answer",
  "githubReason.other": "GitHub couldn't be read",
  "githubReason.feedbackReadFailed": "Couldn't read the checks and review comments.",
  "githubReason.issueBodyReadFailed": "Only the title was filled because the body couldn't be read.",
  "githubReason.unnamedCheck": "Unnamed check",
  "githubReason.fixHere": "Run in a terminal",
} as const;

const ko = {
  "githubReason.not_installed": "gh가 설치되지 않았어요",
  "githubReason.not_logged_in": "gh에 로그인하지 않았어요",
  "githubReason.network_or_rate_limit": "GitHub가 응답하지 않았어요",
  "githubReason.other": "GitHub를 읽지 못했어요",
  "githubReason.feedbackReadFailed": "검사와 리뷰 코멘트를 읽지 못했어요.",
  "githubReason.issueBodyReadFailed": "본문을 불러오지 못해 제목만 넣었어요.",
  "githubReason.unnamedCheck": "이름 없는 검사",
  "githubReason.fixHere": "터미널에서 실행하세요",
} satisfies Record<keyof typeof githubReasonsEnglish, string>;

const zhCN = {
  "githubReason.not_installed": "未安装 gh",
  "githubReason.not_logged_in": "未登录 gh",
  "githubReason.network_or_rate_limit": "GitHub 没有响应",
  "githubReason.other": "无法读取 GitHub",
  "githubReason.feedbackReadFailed": "无法读取检查和评审评论。",
  "githubReason.issueBodyReadFailed": "未能读取正文，仅填入了标题。",
  "githubReason.unnamedCheck": "未命名的检查",
  "githubReason.fixHere": "在终端中运行",
} satisfies Record<keyof typeof githubReasonsEnglish, string>;

const ja = {
  "githubReason.not_installed": "gh がインストールされていません",
  "githubReason.not_logged_in": "gh にログインしていません",
  "githubReason.network_or_rate_limit": "GitHub が応答しませんでした",
  "githubReason.other": "GitHub を読み込めませんでした",
  "githubReason.feedbackReadFailed": "チェックとレビューコメントを読み込めませんでした。",
  "githubReason.issueBodyReadFailed": "本文を読み込めなかったため、タイトルだけを入力しました。",
  "githubReason.unnamedCheck": "名前のないチェック",
  "githubReason.fixHere": "ターミナルで実行してください",
} satisfies Record<keyof typeof githubReasonsEnglish, string>;

export const githubReasonsCatalogs = { en: githubReasonsEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof githubReasonsEnglish>;
