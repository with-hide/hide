import type { Catalog, Catalogs } from "../../schema";

// The left-list guide's words (`practice/guides.ts`).
const en = {
  "practice.left.title": "Reading the left list",
  "practice.left.summary": "Project view and Task view, the states, and what a second line says",
  "practice.left.line": "The same work, read by project or by task. A waiting agent's second line says what it waits for.",
  "practice.left.frame1": "Project view: project › branch or worktree › tab › agent",
  "practice.left.frame2": "Task view groups by state: Needs You · Unread · Working · Seen",
  "practice.left.frame3": "Working: a whole ring works, a dashed ring waits; its second line says for what",
  "practice.left.frame4": "Sleeping · wakes when opened",
};

const ko = {
  "practice.left.title": "왼쪽 목록 읽기",
  "practice.left.summary": "프로젝트 보기, 태스크 보기, 상태, 그리고 둘째 줄이 말하는 것",
  "practice.left.line": "같은 일을 프로젝트별로, 태스크별로 읽어요. 기다리는 에이전트의 둘째 줄이 무엇을 기다리는지 말해 줘요.",
  "practice.left.frame1": "프로젝트 보기: 프로젝트 › 브랜치·워크트리 › 탭 › 에이전트",
  "practice.left.frame2": "태스크 보기는 상태별: 내 차례 · 읽을 것 · 작업 중 · 확인함",
  "practice.left.frame3": "작업 중: 꽉 찬 고리는 작업 중, 점선 고리는 기다리는 중. 둘째 줄이 무엇을 기다리는지 말해요",
  "practice.left.frame4": "재움 · 열면 깨어남",
} satisfies Catalog<typeof en>;

const zhCN = {
  "practice.left.title": "读懂左侧列表",
  "practice.left.summary": "项目视图、任务视图、状态，以及第二行的含义",
  "practice.left.line": "同一份工作可按项目或按任务来读。等待中的智能体的第二行会说明它在等什么。",
  "practice.left.frame1": "项目视图：项目 › 分支或工作树 › 标签页 › 智能体",
  "practice.left.frame2": "任务视图按状态分组：轮到你 · 待读 · 工作中 · 已查看",
  "practice.left.frame3": "工作中：实心圆环在工作，虚线圆环在等待；第二行说明在等什么",
  "practice.left.frame4": "休眠 · 打开时唤醒",
} satisfies Catalog<typeof en>;

const ja = {
  "practice.left.title": "左の一覧の読み方",
  "practice.left.summary": "プロジェクト表示とタスク表示、状態、2 行目の意味",
  "practice.left.line": "同じ仕事をプロジェクト別・タスク別に読みます。待機中のエージェントの 2 行目が、何を待っているかを示します。",
  "practice.left.frame1": "プロジェクト表示: プロジェクト › ブランチ/ワークツリー › タブ › エージェント",
  "practice.left.frame2": "タスク表示は状態別: 自分の番 · 未読 · 作業中 · 確認済み",
  "practice.left.frame3": "作業中: 実線の輪は作業中、破線の輪は待機中。2 行目が待っている対象を示します",
  "practice.left.frame4": "スリープ · 開くと再開",
} satisfies Catalog<typeof en>;

export default { en, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof en>;
