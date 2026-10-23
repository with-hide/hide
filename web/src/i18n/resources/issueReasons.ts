// The words for why a Local issue was not made, changed or read, and why a
// pull request's issue was not made or linked: the core sends a
// `local_issue.<code>` or `pr_link.<code>` with the limit its sentence names
// (docs/LOCALIZATION.md, Translation boundary). The core's own English stays in
// the log; `issueFailure.ts` picks one of these by code, and a code missing here
// shows the headline alone.
import type { Catalogs } from "../schema";

export const issueReasonsEnglish = {
  "issueReason.local_issue.title_empty": "The title is empty.",
  "issueReason.local_issue.title_multiline": "The title has to be one line.",
  "issueReason.local_issue.title_too_long": "A title holds at most {{limit}} characters.",
  "issueReason.local_issue.body_too_long": "A body holds at most {{limit}} KiB.",
  "issueReason.local_issue.projects_full": "Local issues are kept for at most {{limit}} projects.",
  "issueReason.local_issue.issues_full": "A project holds at most {{limit}} local issues.",
  "issueReason.local_issue.missing": "This local issue no longer exists.",
  "issueReason.local_issue.store_unreadable": "Hide could not read the local issue file.",
  "issueReason.local_issue.not_local": "Only a local issue is edited in Hide.",
  "issueReason.local_issue.project_unknown": "Hide could not find this issue's project.",
  "issueReason.pr_link.pr_unknown": "This PR is no longer in the project's list.",
  "issueReason.pr_link.repository_unknown": "Hide does not know this project's repository.",
  "issueReason.pr_link.worker_unavailable": "Hide could not start the work. Try again.",
  "issueReason.pr_link.request_invalid": "Hide could not read the request. Try again.",
  "issueReason.pr_link.issue_not_in_project": "The issue is not in this project.",
  "issueReason.pr_link.no_worktree": "The PR's branch has no worktree. Create a worktree first.",
  "issueReason.pr_link.link_not_saved": "Hide could not save the link.",
  "issueReason.pr_link.create_failed": "GitHub refused the request or did not answer. Try again.",
  "issueReason.pr_link.body_not_written": "GitHub did not take the new PR body. Try again.",
} as const;

const ko = {
  "issueReason.local_issue.title_empty": "제목이 비어 있어요.",
  "issueReason.local_issue.title_multiline": "제목은 한 줄로 써 주세요.",
  "issueReason.local_issue.title_too_long": "제목은 최대 {{limit}}자까지 쓸 수 있어요.",
  "issueReason.local_issue.body_too_long": "본문은 최대 {{limit}} KiB까지 쓸 수 있어요.",
  "issueReason.local_issue.projects_full": "로컬 이슈는 최대 {{limit}}개 프로젝트까지 둘 수 있어요.",
  "issueReason.local_issue.issues_full": "한 프로젝트에 로컬 이슈는 최대 {{limit}}개까지 둘 수 있어요.",
  "issueReason.local_issue.missing": "이 로컬 이슈는 이제 없어요.",
  "issueReason.local_issue.store_unreadable": "로컬 이슈 파일을 읽지 못했어요.",
  "issueReason.local_issue.not_local": "Hide에서는 로컬 이슈만 고칠 수 있어요.",
  "issueReason.local_issue.project_unknown": "이 이슈의 프로젝트를 찾지 못했어요.",
  "issueReason.pr_link.pr_unknown": "이 PR은 이제 프로젝트 목록에 없어요.",
  "issueReason.pr_link.repository_unknown": "이 프로젝트의 저장소를 알 수 없어요.",
  "issueReason.pr_link.worker_unavailable": "작업을 시작하지 못했어요. 다시 시도하세요.",
  "issueReason.pr_link.request_invalid": "요청을 읽지 못했어요. 다시 시도하세요.",
  "issueReason.pr_link.issue_not_in_project": "이 이슈는 이 프로젝트에 없어요.",
  "issueReason.pr_link.no_worktree": "PR 브랜치에 워크트리가 없어요. 먼저 워크트리를 만드세요.",
  "issueReason.pr_link.link_not_saved": "연결을 저장하지 못했어요.",
  "issueReason.pr_link.create_failed": "GitHub가 요청을 거절했거나 응답하지 않았어요. 다시 시도하세요.",
  "issueReason.pr_link.body_not_written": "GitHub가 새 PR 본문을 받지 않았어요. 다시 시도하세요.",
} satisfies Record<keyof typeof issueReasonsEnglish, string>;

const zhCN = {
  "issueReason.local_issue.title_empty": "标题为空。",
  "issueReason.local_issue.title_multiline": "标题只能有一行。",
  "issueReason.local_issue.title_too_long": "标题最多 {{limit}} 个字符。",
  "issueReason.local_issue.body_too_long": "正文最多 {{limit}} KiB。",
  "issueReason.local_issue.projects_full": "本地议题最多保存 {{limit}} 个项目。",
  "issueReason.local_issue.issues_full": "一个项目最多有 {{limit}} 个本地议题。",
  "issueReason.local_issue.missing": "此本地议题已不存在。",
  "issueReason.local_issue.store_unreadable": "Hide 无法读取本地议题文件。",
  "issueReason.local_issue.not_local": "在 Hide 中只能编辑本地议题。",
  "issueReason.local_issue.project_unknown": "Hide 找不到此议题所属的项目。",
  "issueReason.pr_link.pr_unknown": "此 PR 已不在项目列表中。",
  "issueReason.pr_link.repository_unknown": "Hide 不知道此项目的仓库。",
  "issueReason.pr_link.worker_unavailable": "Hide 无法开始这项工作。请重试。",
  "issueReason.pr_link.request_invalid": "Hide 无法读取此请求。请重试。",
  "issueReason.pr_link.issue_not_in_project": "此议题不在这个项目中。",
  "issueReason.pr_link.no_worktree": "PR 的分支没有工作树。请先创建工作树。",
  "issueReason.pr_link.link_not_saved": "Hide 无法保存关联。",
  "issueReason.pr_link.create_failed": "GitHub 拒绝了请求或没有响应。请重试。",
  "issueReason.pr_link.body_not_written": "GitHub 未接受新的 PR 正文。请重试。",
} satisfies Record<keyof typeof issueReasonsEnglish, string>;

const ja = {
  "issueReason.local_issue.title_empty": "タイトルが空です。",
  "issueReason.local_issue.title_multiline": "タイトルは 1 行で入力してください。",
  "issueReason.local_issue.title_too_long": "タイトルは最大 {{limit}} 文字です。",
  "issueReason.local_issue.body_too_long": "本文は最大 {{limit}} KiB です。",
  "issueReason.local_issue.projects_full": "ローカル Issue を保存できるプロジェクトは最大 {{limit}} 件です。",
  "issueReason.local_issue.issues_full": "1 つのプロジェクトに置けるローカル Issue は最大 {{limit}} 件です。",
  "issueReason.local_issue.missing": "このローカル Issue はもうありません。",
  "issueReason.local_issue.store_unreadable": "ローカル Issue のファイルを読み込めませんでした。",
  "issueReason.local_issue.not_local": "Hide で編集できるのはローカル Issue だけです。",
  "issueReason.local_issue.project_unknown": "この Issue のプロジェクトが見つかりませんでした。",
  "issueReason.pr_link.pr_unknown": "この PR はもうプロジェクトの一覧にありません。",
  "issueReason.pr_link.repository_unknown": "このプロジェクトのリポジトリがわかりません。",
  "issueReason.pr_link.worker_unavailable": "作業を開始できませんでした。もう一度お試しください。",
  "issueReason.pr_link.request_invalid": "リクエストを読み取れませんでした。もう一度お試しください。",
  "issueReason.pr_link.issue_not_in_project": "この Issue はこのプロジェクトにありません。",
  "issueReason.pr_link.no_worktree": "PR のブランチにワークツリーがありません。先にワークツリーを作成してください。",
  "issueReason.pr_link.link_not_saved": "関連付けを保存できませんでした。",
  "issueReason.pr_link.create_failed": "GitHub がリクエストを拒否したか、応答しませんでした。もう一度お試しください。",
  "issueReason.pr_link.body_not_written": "GitHub が新しい PR 本文を受け付けませんでした。もう一度お試しください。",
} satisfies Record<keyof typeof issueReasonsEnglish, string>;

export const issueReasonsCatalogs = { en: issueReasonsEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof issueReasonsEnglish>;
