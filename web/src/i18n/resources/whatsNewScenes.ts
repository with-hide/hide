import type { Catalog, Catalogs } from "../schema";

// The words a What's new scene plays (`web/src/components/whats-new-scenes/`):
// the example work a scene shows, never the operator's own. The names a scene
// shares with the screen it shows (the automation screen's title, an example
// automation) come from that screen's own catalog.
export const whatsNewScenesEnglish = {
  "whatsNewScene.automation.night": "It waits through the night and starts at 9:00",
  "whatsNewScene.automation.next": "Tomorrow 09:00",
  "whatsNewScene.automation.line1": "Reading 23 new issues…",
  "whatsNewScene.automation.line2": "Install fails on Windows → bug, high",
  "whatsNewScene.automation.line3": "Dark mode on the phone → request",
  "whatsNewScene.automation.line4": "Same as an earlier issue → duplicate",
  "whatsNewScene.automation.line5": "Done · 4 bugs, 7 requests, 3 duplicates",
  "whatsNewScene.automation.finished": "{{name}} finished · {{time}}",
};

const ko: Catalog<typeof whatsNewScenesEnglish> = {
  "whatsNewScene.automation.night": "밤새 기다렸다가 아침 9시에 시작해요",
  "whatsNewScene.automation.next": "내일 09:00",
  "whatsNewScene.automation.line1": "새 이슈 23개 읽는 중…",
  "whatsNewScene.automation.line2": "Windows에서 설치 실패 → 버그, 높음",
  "whatsNewScene.automation.line3": "휴대폰 다크 모드 → 요청",
  "whatsNewScene.automation.line4": "이미 있는 이슈와 같음 → 중복",
  "whatsNewScene.automation.line5": "끝 · 버그 4, 요청 7, 중복 3",
  "whatsNewScene.automation.finished": "{{name}} 끝났어요 · {{time}}",
};

const zh: Catalog<typeof whatsNewScenesEnglish> = {
  "whatsNewScene.automation.night": "整夜等待，早上 9 点开始",
  "whatsNewScene.automation.next": "明天 09:00",
  "whatsNewScene.automation.line1": "正在读取 23 个新问题…",
  "whatsNewScene.automation.line2": "Windows 上安装失败 → 缺陷，高",
  "whatsNewScene.automation.line3": "手机深色模式 → 需求",
  "whatsNewScene.automation.line4": "与已有问题相同 → 重复",
  "whatsNewScene.automation.line5": "完成 · 缺陷 4，需求 7，重复 3",
  "whatsNewScene.automation.finished": "{{name}} 已完成 · {{time}}",
};

const ja: Catalog<typeof whatsNewScenesEnglish> = {
  "whatsNewScene.automation.night": "夜のあいだ待って、朝 9 時に始めます",
  "whatsNewScene.automation.next": "明日 09:00",
  "whatsNewScene.automation.line1": "新しい Issue 23 件を読んでいます…",
  "whatsNewScene.automation.line2": "Windows でインストール失敗 → バグ、高",
  "whatsNewScene.automation.line3": "スマホのダークモード → 要望",
  "whatsNewScene.automation.line4": "既存の Issue と同じ → 重複",
  "whatsNewScene.automation.line5": "完了 · バグ 4、要望 7、重複 3",
  "whatsNewScene.automation.finished": "{{name}} が終わりました · {{time}}",
};

export const whatsNewScenesCatalogs = { en: whatsNewScenesEnglish, ko, "zh-CN": zh, ja } satisfies Catalogs<typeof whatsNewScenesEnglish>;
