import type { Catalog, Catalogs } from "../schema";

export const boardEnglish = {
  "board.reading": "Reading…",
  "board.empty": "No open issues",
  "board.review.required": "Review required",
  "board.review.changes": "Changes requested",
  "board.review.approved": "Approved",
  "board.sourceReadFailed": "{{source}} read failed",
  "board.prLineage": "Issue and agent lineage of this PR",
  "board.showOnPrsTab": "Open the PR card",
  "board.openPr": "Open PR",
} as const;

const ko = {
  "board.reading": "불러오는 중…",
  "board.empty": "열린 이슈가 없어요",
  "board.review.required": "리뷰 필요",
  "board.review.changes": "변경 요청",
  "board.review.approved": "승인됨",
  "board.sourceReadFailed": "{{source}} 불러오기 실패",
  "board.prLineage": "이 PR의 이슈와 에이전트 계보",
  "board.showOnPrsTab": "PR 카드 열기",
  "board.openPr": "PR 열기",
} satisfies Catalog<typeof boardEnglish>;

const zhCN = {
  "board.reading": "正在读取…",
  "board.empty": "暂无未关闭的议题",
  "board.review.required": "需要审查",
  "board.review.changes": "请求修改",
  "board.review.approved": "已批准",
  "board.sourceReadFailed": "未能读取 {{source}}",
  "board.prLineage": "此 PR 的议题和智能体谱系",
  "board.showOnPrsTab": "打开 PR 卡片",
  "board.openPr": "打开 PR",
} satisfies Catalog<typeof boardEnglish>;

const ja = {
  "board.reading": "読み込み中…",
  "board.empty": "オープンなIssueはありません",
  "board.review.required": "レビューが必要",
  "board.review.changes": "変更をリクエスト",
  "board.review.approved": "承認済み",
  "board.sourceReadFailed": "{{source}}の読み込みに失敗",
  "board.prLineage": "この PR の Issue とエージェントの系譜",
  "board.showOnPrsTab": "PR カードを開く",
  "board.openPr": "PR を開く",
} satisfies Catalog<typeof boardEnglish>;

export const boardCatalogs = { en: boardEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof boardEnglish>;
