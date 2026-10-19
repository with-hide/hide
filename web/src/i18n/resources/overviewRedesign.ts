import type { Catalogs } from "../schema";

const words = {
  "overviewRedesign.work": ["Work", "작업", "作業", "工作"],
  "overviewRedesign.history": ["History", "기록", "履歴", "记录"],
  "overviewRedesign.accountScope": ["All accounts · all projects", "계정 전체 · 모든 프로젝트", "全アカウント · 全プロジェクト", "全部账户 · 所有项目"],
  "overviewRedesign.projectHome": ["Project home", "프로젝트 홈", "プロジェクトホーム", "项目主页"],
  "overviewRedesign.checkouts": ["Checkouts", "체크아웃", "チェックアウト", "检出"],
  "overviewRedesign.sessions": ["Sessions", "세션", "セッション", "会话"],
  "overviewRedesign.sessionFamily": ["{{roots}} roots · {{children}} children", "부모 {{roots}} · 자식 {{children}}", "親 {{roots}} · 子 {{children}}", "父 {{roots}} · 子 {{children}}"],
  "overviewRedesign.noSessions": ["No live sessions", "실행 중인 세션이 없습니다", "実行中のセッションはありません", "没有运行中的会话"],
  "overviewRedesign.noCheckouts": ["No available checkouts", "열 수 있는 체크아웃이 없습니다", "開けるチェックアウトはありません", "没有可打开的检出"],
  "overviewRedesign.allProjects": ["Back to all projects", "모든 프로젝트로", "すべてのプロジェクトへ", "返回所有项目"],
  "overviewRedesign.openCheckout": ["Open {{checkout}}", "{{checkout}} 열기", "{{checkout}} を開く", "打开 {{checkout}}"],
  "overviewRedesign.rootTasks": ["Root tasks", "루트 작업", "ルート作業", "根任务"],
  "overviewRedesign.mixedBacklog": ["GitHub {{github}} · Memos {{local}}", "GitHub {{github}} · 메모 {{local}}", "GitHub {{github}} · メモ {{local}}", "GitHub {{github}} · 备忘 {{local}}"],
} as const;

export const overviewRedesignEnglish = Object.fromEntries(Object.entries(words).map(([key, value]) => [key, value[0]])) as { [K in keyof typeof words]: string };
export const overviewRedesignCatalogs = Object.fromEntries((["en", "ko", "ja", "zh-CN"] as const).map((language, i) => [language, Object.fromEntries(Object.entries(words).map(([key, value]) => [key, value[i]!]))])) as Catalogs<typeof overviewRedesignEnglish>;
