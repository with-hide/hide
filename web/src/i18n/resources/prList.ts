import type { Catalog, Catalogs } from "../schema";

export const prListEnglish = {
  "prList.takeHint": "Assign to an agent\nOpen the Start dialog on the PR branch. First instruction = failed checks and review comments",
  "prList.linkHint": "Link issue\nLink this PR to an issue. If none is available, create one from the PR title and body",
  "prList.cleanHint": "Clean up\nRemove merged worktrees or those with missing folders, and their resting agents. A confirmation dialog appears first",
  "prList.gitHubHint": "GitHub\nReview and merge",
  "prList.openIssue": "Open {{issue}} on GitHub",
  "prList.linkIssue": "Link issue",
  "prList.searchIssues": "Search issues",
  "prList.newIssue": "Create new issue",
  "prList.delegate": "Assign",
  "prList.actions": "PR actions",
  "prList.copyBranch": "Copy branch name",
} as const;

const ko = {
  "prList.takeHint": "에이전트에게 맡기기\nPR 브랜치에서 시작 대화상자를 열어요. 첫 지시에는 실패한 검사와 리뷰 코멘트가 들어가요",
  "prList.linkHint": "이슈 연결\n이 PR을 이슈에 연결해요. 연결할 이슈가 없으면 PR 제목과 본문으로 새 이슈를 만들어요",
  "prList.cleanHint": "정리\n머지됐거나 폴더가 없는 워크트리와 거기서 쉬는 에이전트를 지워요. 먼저 확인 대화상자가 떠요",
  "prList.gitHubHint": "GitHub\n리뷰하고 머지",
  "prList.openIssue": "GitHub에서 {{issue}} 열기",
  "prList.linkIssue": "이슈 연결",
  "prList.searchIssues": "이슈 검색",
  "prList.newIssue": "새 이슈 만들기",
  "prList.delegate": "맡기기",
  "prList.actions": "PR 동작",
  "prList.copyBranch": "브랜치 이름 복사",
} satisfies Catalog<typeof prListEnglish>;

const zhCN = {
  "prList.takeHint": "交给智能体\n在 PR 分支上打开启动对话框。初始指令 = 失败的检查和审查评论",
  "prList.linkHint": "关联议题\n将此 PR 关联到议题。如果没有可关联的议题，则根据 PR 标题和正文创建新议题",
  "prList.cleanHint": "清理\n移除已合并或文件夹缺失的工作树，以及其中空闲的智能体。操作前会显示确认对话框",
  "prList.gitHubHint": "GitHub\n审查并合并",
  "prList.openIssue": "在 GitHub 上打开 {{issue}}",
  "prList.linkIssue": "关联议题",
  "prList.searchIssues": "搜索议题",
  "prList.newIssue": "创建新议题",
  "prList.delegate": "交给智能体",
  "prList.actions": "PR 操作",
  "prList.copyBranch": "复制分支名称",
} satisfies Catalog<typeof prListEnglish>;

const ja = {
  "prList.takeHint": "エージェントに任せる\nPR のブランチで開始ダイアログを開きます。最初の指示 = 失敗したチェックとレビューコメント",
  "prList.linkHint": "Issue を関連付ける\nこの PR を Issue に関連付けます。該当する Issue がなければ PR のタイトルと本文から作成します",
  "prList.cleanHint": "整理\nマージ済み、またはフォルダーのないワークツリーと、そこで待機中のエージェントを削除します。先に確認ダイアログが表示されます",
  "prList.gitHubHint": "GitHub\nレビューしてマージ",
  "prList.openIssue": "GitHub で{{issue}}を開く",
  "prList.linkIssue": "Issue を関連付ける",
  "prList.searchIssues": "Issue を検索",
  "prList.newIssue": "新しい Issue を作成",
  "prList.delegate": "任せる",
  "prList.actions": "PR の操作",
  "prList.copyBranch": "ブランチ名をコピー",
} satisfies Catalog<typeof prListEnglish>;

export const prListCatalogs = { en: prListEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof prListEnglish>;
