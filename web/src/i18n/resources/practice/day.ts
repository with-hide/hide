import type { Catalog, Catalogs } from "../../schema";

// The start-of-day guide's words (`practice/guides.ts`).
const en = {
  "practice.day.title": "Start the day from the Overview",
  "practice.day.summary": "See where everything stands, pick the next work, and bring yesterday's work back",
  "practice.day.line": "The Overview shows where everything stands; History finds yesterday's work and brings it back.",
  "practice.day.frame1": "Overview: Work, Projects, Backlog, History, Usage",
  "practice.day.frame2": "Find yesterday's work by a word you remember",
  "practice.day.frame3": "Bring the session back and carry on",
};

const ko = {
  "practice.day.title": "개요로 하루 시작하기",
  "practice.day.summary": "지금 상황을 보고, 다음 일을 고르고, 어제 일을 되살려요",
  "practice.day.line": "개요가 지금 상황을 보여 주고, 기록에서 어제 일을 찾아 되살려요.",
  "practice.day.frame1": "개요: 작업, 프로젝트, 백로그, 기록, 사용량",
  "practice.day.frame2": "기억나는 낱말로 어제 일을 찾아요",
  "practice.day.frame3": "세션을 되살려 이어서 해요",
} satisfies Catalog<typeof en>;

const zhCN = {
  "practice.day.title": "从概览开始一天",
  "practice.day.summary": "看清现状、挑下一件事，并让昨天的工作回来",
  "practice.day.line": "概览显示现在的情况；历史能找到昨天的工作并让它回来。",
  "practice.day.frame1": "概览：工作、项目、待办、历史、用量",
  "practice.day.frame2": "用记得的词找到昨天的工作",
  "practice.day.frame3": "让会话回来，接着做",
} satisfies Catalog<typeof en>;

const ja = {
  "practice.day.title": "概要から一日を始める",
  "practice.day.summary": "今の状況を見て、次の作業を選び、昨日の作業を戻します",
  "practice.day.line": "概要が今の状況を見せ、履歴から昨日の作業を探して戻せます。",
  "practice.day.frame1": "概要: 作業、プロジェクト、バックログ、履歴、使用量",
  "practice.day.frame2": "覚えている語で昨日の作業を探します",
  "practice.day.frame3": "セッションを戻して続けます",
} satisfies Catalog<typeof en>;

export default { en, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof en>;
