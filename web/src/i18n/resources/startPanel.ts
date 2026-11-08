import type { Catalogs } from "../schema";

// The start panel's where row (which project, which branch) and the Agent tab
// bar's new-tab menu. Each row is English, Korean, Japanese, Simplified Chinese.
const words = {
  "startPanel.project": ["Project", "프로젝트", "プロジェクト", "项目"],
  "startPanel.branch": ["Branch", "브랜치", "ブランチ", "分支"],
  "startPanel.findProject": ["Find a project", "프로젝트 찾기", "プロジェクトを検索", "查找项目"],
  "startPanel.findBranch": ["Find a branch", "브랜치 찾기", "ブランチを検索", "查找分支"],
  "startPanel.noMatch": ["Nothing matches", "맞는 항목이 없어요", "一致する項目がありません", "没有匹配项"],
  "startPanel.deviceOffline": ["{{device}} · not connected", "{{device}} · 연결 안 됨", "{{device}} · 未接続", "{{device}} · 未连接"],
  "startPanel.mainCheckout": ["main", "기본", "メイン", "主"],
  "startPanel.current": ["current", "지금", "現在", "当前"],
  "startPanel.newBranch": ["Start on a new branch…", "새 브랜치에서 시작…", "新しいブランチで開始…", "在新分支上开始…"],
  "startPanel.newBranchName": ["New branch name", "새 브랜치 이름", "新しいブランチ名", "新分支名称"],
  "startPanel.from": ["from", "기준", "元", "基于"],
  "startPanel.cancelNewBranch": ["Back to the chosen branch", "고른 브랜치로 돌아가기", "選んだブランチに戻る", "返回所选分支"],
  "startPanel.newMenu": ["New tab options", "새 탭 고르기", "新しいタブの種類", "新标签页选项"],
  "startPanel.terminal": ["Terminal", "터미널", "ターミナル", "终端"],
  "startPanel.startAgent": ["Start an agent…", "에이전트 시작…", "エージェントを開始…", "启动智能体…"],
} as const;

export const startPanelEnglish = Object.fromEntries(Object.entries(words).map(([key, value]) => [key, value[0]])) as { [K in keyof typeof words]: string };
export const startPanelCatalogs = Object.fromEntries((["en", "ko", "ja", "zh-CN"] as const).map((language, i) => [language, Object.fromEntries(Object.entries(words).map(([key, value]) => [key, value[i]!]))])) as Catalogs<typeof startPanelEnglish>;
