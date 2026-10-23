import type { InterfaceLanguage } from "../locale";
import type { Catalog, Catalogs, MessageSchema } from "../schema";

// Practice mode: the guide over the real shell. Each chapter's words are a
// file of their own in `practice/`, named like the chapter, every key under
// `practice.<chapter>.`; they join the catalog here without being listed.

const chapterFiles = import.meta.glob<Catalogs<MessageSchema>>("./practice/*.ts", { eager: true, import: "default" });

/** Every chapter file's words in `language`; a key outside its file's chapter would overwrite another's. */
function chapterWords(language: InterfaceLanguage): Record<string, string> {
  return Object.assign({}, ...Object.entries(chapterFiles).map(([path, catalogs]) => {
    const prefix = `practice.${path.slice(path.lastIndexOf("/") + 1, -".ts".length)}.`;
    const words = catalogs[language];
    const stray = Object.keys(words).find((key) => !key.startsWith(prefix));
    if (stray) throw new Error(`Practice words ${stray} are in ${path}`);
    return words;
  }));
}

const common = {
  "practice.mode": "Practice · Chapter {{number}} {{title}}",
  "practice.count": "{{index}} / {{total}}",
  "practice.next": "Next",
  "practice.try": "Try it",
  "practice.back": "Back",
  "practice.toChapter": "Chapter {{number}}",
  "practice.toHub": "All chapters",
  "practice.hub": "← All chapters",
  "practice.hubTitle": "Practice Hide",
  "practice.hubLead": "Try Hide on a practice screen. No real agent, project or file is touched.",
  "practice.hubKeys": "↑↓ to choose · ⏎ to start",
  "practice.hubUnsaved": "Progress is saved while a Hide window is open.",
  "practice.chapter": "Chapter {{number}}",
  "practice.steps": "{{count}} steps",
  "practice.status.new": "New",
  "practice.status.paused": "Stopped at {{index}} / {{total}}",
  "practice.status.done": "✓ Seen",
  "practice.status.changed": "Changed since",
  "practice.start": "Start",
  "practice.continue": "Continue",
  "practice.replay": "See again",
  "practice.fromStart": "From the start",
  "practice.via.press": "Press where it's marked",
  "practice.questHead": "Practice · Chapter {{number}}",
  "practice.questRestart": "↺ Start over",
  "practice.via.key": "Or press {{keys}}",
  "practice.via.buttonOrKey": "Press the button, or {{keys}}",
  "practice.via.drag": "Or drag the pane's header yourself",
  "practice.zone.view": "① Where you look",
  "practice.zone.viewSub": "Choose what to see",
  "practice.zone.work": "② Where work happens",
  "practice.zone.workSub": "Agent terminals",
  "practice.zone.project": "③ The project and its work",
  "practice.zone.projectSub": "Status · To do · Files",
};

export const practiceEnglish: typeof common = { ...common, ...chapterWords("en") };

const ko = {
  "practice.mode": "연습 모드 · {{number}}편 {{title}}",
  "practice.count": "{{index}} / {{total}}",
  "practice.next": "다음",
  "practice.try": "해 보기",
  "practice.back": "이전",
  "practice.toChapter": "{{number}}편으로",
  "practice.toHub": "장 목록으로",
  "practice.hub": "← 장 목록",
  "practice.hubTitle": "Hide 연습",
  "practice.hubLead": "연습 화면에서 직접 눌러 보며 익혀요. 실제 에이전트·프로젝트·파일은 건드리지 않아요.",
  "practice.hubKeys": "↑↓ 고르기 · ⏎ 시작",
  "practice.hubUnsaved": "진행 기록은 Hide 창이 열려 있을 때 저장돼요.",
  "practice.chapter": "{{number}}편",
  "practice.steps": "{{count}}단계",
  "practice.status.new": "새로",
  "practice.status.paused": "{{index}} / {{total}}에서 멈춤",
  "practice.status.done": "✓ 봄",
  "practice.status.changed": "바뀜",
  "practice.start": "시작",
  "practice.continue": "이어서",
  "practice.replay": "다시 보기",
  "practice.fromStart": "처음부터",
  "practice.via.press": "표시된 곳을 누르세요",
  "practice.questHead": "연습 모드 · {{number}}편",
  "practice.questRestart": "↺ 처음부터",
  "practice.via.key": "단축키 {{keys}} 를 눌러도 돼요",
  "practice.via.buttonOrKey": "버튼을 누르거나 단축키 {{keys}}",
  "practice.via.drag": "pane 머리줄을 직접 끌어도 돼요",
  "practice.zone.view": "① 보는 곳",
  "practice.zone.viewSub": "무엇을 볼지 고르기",
  "practice.zone.work": "② 일하는 곳",
  "practice.zone.workSub": "에이전트 터미널",
  "practice.zone.project": "③ 프로젝트와 그 작업",
  "practice.zone.projectSub": "현황 · 할 일 · 파일",
  ...chapterWords("ko"),
} satisfies Catalog<typeof practiceEnglish>;

const zhCN = {
  "practice.mode": "练习模式 · 第 {{number}} 章 {{title}}",
  "practice.count": "{{index}} / {{total}}",
  "practice.next": "下一步",
  "practice.try": "试一试",
  "practice.back": "上一步",
  "practice.toChapter": "进入第 {{number}} 章",
  "practice.toHub": "全部章节",
  "practice.hub": "← 全部章节",
  "practice.hubTitle": "练习 Hide",
  "practice.hubLead": "在练习界面上亲手试一试。不会碰到任何真实的智能体、项目或文件。",
  "practice.hubKeys": "↑↓ 选择 · ⏎ 开始",
  "practice.hubUnsaved": "Hide 窗口打开时才会保存进度。",
  "practice.chapter": "第 {{number}} 章",
  "practice.steps": "{{count}} 步",
  "practice.status.new": "新",
  "practice.status.paused": "停在 {{index}} / {{total}}",
  "practice.status.done": "✓ 已看",
  "practice.status.changed": "已更新",
  "practice.start": "开始",
  "practice.continue": "继续",
  "practice.replay": "再看一遍",
  "practice.fromStart": "从头开始",
  "practice.via.press": "按一下标出的地方",
  "practice.questHead": "练习模式 · 第 {{number}} 章",
  "practice.questRestart": "↺ 重新开始",
  "practice.via.key": "也可以按 {{keys}}",
  "practice.via.buttonOrKey": "点按钮，或按 {{keys}}",
  "practice.via.drag": "也可以直接拖动窗格标题栏",
  "practice.zone.view": "① 查看区",
  "practice.zone.viewSub": "选择要看的内容",
  "practice.zone.work": "② 工作区",
  "practice.zone.workSub": "代理终端",
  "practice.zone.project": "③ 项目与工作",
  "practice.zone.projectSub": "概况 · 待办 · 文件",
  ...chapterWords("zh-CN"),
} satisfies Catalog<typeof practiceEnglish>;

const ja = {
  "practice.mode": "練習モード · 第{{number}}章 {{title}}",
  "practice.count": "{{index}} / {{total}}",
  "practice.next": "次へ",
  "practice.try": "やってみる",
  "practice.back": "戻る",
  "practice.toChapter": "第{{number}}章へ",
  "practice.toHub": "章の一覧へ",
  "practice.hub": "← 章の一覧",
  "practice.hubTitle": "Hide の練習",
  "practice.hubLead": "練習用の画面で実際に押して覚えます。本物のエージェント・プロジェクト・ファイルには触れません。",
  "practice.hubKeys": "↑↓ で選ぶ · ⏎ で始める",
  "practice.hubUnsaved": "進み具合は Hide のウィンドウが開いているときに保存されます。",
  "practice.chapter": "第 {{number}} 章",
  "practice.steps": "{{count}} ステップ",
  "practice.status.new": "新しい",
  "practice.status.paused": "{{index}} / {{total}} で中断",
  "practice.status.done": "✓ 見た",
  "practice.status.changed": "更新あり",
  "practice.start": "始める",
  "practice.continue": "続きから",
  "practice.replay": "もう一度見る",
  "practice.fromStart": "最初から",
  "practice.via.press": "示された場所を押してください",
  "practice.questHead": "練習モード · 第{{number}}章",
  "practice.questRestart": "↺ 最初から",
  "practice.via.key": "{{keys}} を押してもできます",
  "practice.via.buttonOrKey": "ボタンを押すか {{keys}}",
  "practice.via.drag": "ペインのヘッダーを直接ドラッグしてもできます",
  "practice.zone.view": "① 見るところ",
  "practice.zone.viewSub": "何を見るか選ぶ",
  "practice.zone.work": "② 作業するところ",
  "practice.zone.workSub": "エージェントのターミナル",
  "practice.zone.project": "③ プロジェクトとその作業",
  "practice.zone.projectSub": "状況 · やること · ファイル",
  ...chapterWords("ja"),
} satisfies Catalog<typeof practiceEnglish>;

export const practiceCatalogs = { en: practiceEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof practiceEnglish>;
