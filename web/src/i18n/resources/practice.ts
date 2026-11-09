import type { Catalog, Catalogs } from "../schema";

// Practice mode: the first-run cards over the real shell and the flow cards that
// look things up. Each flow's words are a file of their own in `practice/`,
// which the practice window adds when it loads (`web/src/practice/words.ts`);
// the desktop host and the shell never show them.

export const practiceEnglish = {
  "practice.mode": "Practice · Flow {{number}} {{title}}",
  "practice.leave": "Back to Hide",
  "practice.count": "{{index}} / {{total}}",
  "practice.next": "Next",
  "practice.press": "Press it for me",
  "practice.back": "Back",
  "practice.hub": "← All flows",
  "practice.hubTitle": "Practice Hide",
  "practice.hubLead": "Look around on a practice screen, then use the cards to look things up any time. Nothing real is touched.",
  "practice.hubAgain": "Help (?) > Practice Hide opens this again any time.",
  "practice.hubKeys": "Arrows to choose · ⏎ to open",
  "practice.hubUnsaved": "Progress is saved while a Hide window is open.",
  "practice.chapter": "Flow {{number}}",
  "practice.steps": "{{count}} steps",
  "practice.status.new": "New",
  "practice.status.done": "✓ Seen",
  "practice.status.changed": "Changed since",
  "practice.start": "Start",
  "practice.replay": "See again",
};

const ko = {
  "practice.mode": "연습 모드 · 흐름 {{number}} {{title}}",
  "practice.leave": "Hide로 돌아가기",
  "practice.count": "{{index}} / {{total}}",
  "practice.next": "다음",
  "practice.press": "대신 누르기",
  "practice.back": "이전",
  "practice.hub": "← 흐름 목록",
  "practice.hubTitle": "Hide 연습",
  "practice.hubLead": "연습 화면에서 먼저 둘러보고, 나머지는 카드로 언제든 찾아봐요. 실제 에이전트·프로젝트·파일은 건드리지 않아요.",
  "practice.hubAgain": "언제든 도움말(?) > Hide 연습하기에서 다시 열 수 있어요.",
  "practice.hubKeys": "화살표로 고르기 · ⏎ 열기",
  "practice.hubUnsaved": "진행 기록은 Hide 창이 열려 있을 때 저장돼요.",
  "practice.chapter": "흐름 {{number}}",
  "practice.steps": "{{count}}단계",
  "practice.status.new": "새로",
  "practice.status.done": "✓ 봄",
  "practice.status.changed": "바뀜",
  "practice.start": "시작",
  "practice.replay": "다시 보기",
} satisfies Catalog<typeof practiceEnglish>;

const zhCN = {
  "practice.mode": "练习模式 · 流程 {{number}} {{title}}",
  "practice.leave": "返回 Hide",
  "practice.count": "{{index}} / {{total}}",
  "practice.next": "下一步",
  "practice.press": "替我点",
  "practice.back": "上一步",
  "practice.hub": "← 全部流程",
  "practice.hubTitle": "练习 Hide",
  "practice.hubLead": "先在练习画面里看一圈，之后随时用卡片查阅。不会动任何真实的智能体、项目或文件。",
  "practice.hubAgain": "随时可以从 帮助(?) > 练习 Hide 再次打开。",
  "practice.hubKeys": "方向键选择 · ⏎ 打开",
  "practice.hubUnsaved": "Hide 窗口打开时才会保存进度。",
  "practice.chapter": "流程 {{number}}",
  "practice.steps": "{{count}} 步",
  "practice.status.new": "新",
  "practice.status.done": "✓ 已看",
  "practice.status.changed": "已更新",
  "practice.start": "开始",
  "practice.replay": "再看一遍",
} satisfies Catalog<typeof practiceEnglish>;

const ja = {
  "practice.mode": "練習モード · フロー{{number}} {{title}}",
  "practice.leave": "Hide に戻る",
  "practice.count": "{{index}} / {{total}}",
  "practice.next": "次へ",
  "practice.press": "代わりに押す",
  "practice.back": "戻る",
  "practice.hub": "← フローの一覧",
  "practice.hubTitle": "Hide の練習",
  "practice.hubLead": "まず練習画面で見て回り、あとはカードでいつでも調べられます。実際のエージェント・プロジェクト・ファイルには触れません。",
  "practice.hubAgain": "いつでも ヘルプ(?) > Hide を練習する から開き直せます。",
  "practice.hubKeys": "矢印キーで選ぶ · ⏎ で開く",
  "practice.hubUnsaved": "進み具合は Hide のウィンドウが開いているときに保存されます。",
  "practice.chapter": "フロー {{number}}",
  "practice.steps": "{{count}} ステップ",
  "practice.status.new": "新しい",
  "practice.status.done": "✓ 見た",
  "practice.status.changed": "更新あり",
  "practice.start": "始める",
  "practice.replay": "もう一度見る",
} satisfies Catalog<typeof practiceEnglish>;

export const practiceCatalogs = { en: practiceEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof practiceEnglish>;
