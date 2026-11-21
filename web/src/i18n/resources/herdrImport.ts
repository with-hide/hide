import type { Catalog, Catalogs } from "../schema";

// Importing this machine's Herdr save into Hide's own engine (PRD
// engine-bundle-4-herdr-removal D-7, D-8): the sidebar line until it is
// imported, the read-only import in the first run and Settings, and the line
// an import that left something out or could not read the save leaves.
export const herdrImportEnglish = {
  "herdrImport.ask.moving": "Moving…",
  "herdrImport.line.import": "Herdr has {{count}} tabs",
  "herdrImport.line.importAction": "Import",
  "herdrImport.line.running": "Conversations open in Herdr open here as shells",
  "herdrImport.line.unreadable": "Couldn't read the Herdr session",
  "herdrImport.line.retry": "Try again",
  "herdrImport.line.refused": "{{count}} Herdr workspaces weren't moved",
  "herdrImport.line.refusedRow": "{{label}}: {{reason}}",
  "herdrImport.line.done": "OK",
  "herdrImport.line.waiting": "Herdr closed: {{count}} conversations can resume here",
  "herdrImport.line.waitingAction": "Resume",
  "herdrImport.import.title": "Import Herdr work",
  "herdrImport.import.deviceTitle": "Import Herdr work from {{device}}",
  "herdrImport.import.note": "{{count}} tabs open with the same layout and folders. Herdr is left as it is.",
  "herdrImport.import.button": "Import",
} as const;

const ko = {
  "herdrImport.ask.moving": "옮기는 중…",
  "herdrImport.line.import": "Herdr 탭 {{count}}개",
  "herdrImport.line.importAction": "가져오기",
  "herdrImport.line.running": "Herdr에서 열려 있는 대화는 셸로 열려요",
  "herdrImport.line.unreadable": "Herdr 저장본을 읽지 못했어요",
  "herdrImport.line.retry": "다시 시도",
  "herdrImport.line.refused": "옮기지 못한 Herdr 작업공간 {{count}}개",
  "herdrImport.line.refusedRow": "{{label}}: {{reason}}",
  "herdrImport.line.done": "확인",
  "herdrImport.line.waiting": "Herdr가 닫혀서 대화 {{count}}개를 여기서 이어 갈 수 있어요",
  "herdrImport.line.waitingAction": "이어 가기",
  "herdrImport.import.title": "Herdr 작업 가져오기",
  "herdrImport.import.deviceTitle": "{{device}}의 Herdr 작업 가져오기",
  "herdrImport.import.note": "탭 {{count}}개가 같은 배치와 폴더로 열려요. Herdr는 그대로 둬요.",
  "herdrImport.import.button": "가져오기",
} satisfies Catalog<typeof herdrImportEnglish>;

const zhCN = {
  "herdrImport.ask.moving": "正在迁移…",
  "herdrImport.line.import": "Herdr 有 {{count}} 个标签页",
  "herdrImport.line.importAction": "导入",
  "herdrImport.line.running": "在 Herdr 中打开的对话会在这里以 shell 打开",
  "herdrImport.line.unreadable": "无法读取 Herdr 会话",
  "herdrImport.line.retry": "重试",
  "herdrImport.line.refused": "{{count}} 个 Herdr 工作区未能迁移",
  "herdrImport.line.refusedRow": "{{label}}：{{reason}}",
  "herdrImport.line.done": "好",
  "herdrImport.line.waiting": "Herdr 已关闭：{{count}} 个对话可以在这里继续",
  "herdrImport.line.waitingAction": "继续",
  "herdrImport.import.title": "导入 Herdr 工作",
  "herdrImport.import.deviceTitle": "导入 {{device}} 上的 Herdr 工作",
  "herdrImport.import.note": "{{count}} 个标签页会以相同的布局和文件夹打开。Herdr 保持不变。",
  "herdrImport.import.button": "导入",
} satisfies Catalog<typeof herdrImportEnglish>;

const ja = {
  "herdrImport.ask.moving": "移しています…",
  "herdrImport.line.import": "Herdr のタブ {{count}} 個",
  "herdrImport.line.importAction": "取り込む",
  "herdrImport.line.running": "Herdr で開いている会話はシェルとして開きます",
  "herdrImport.line.unreadable": "Herdr のセッションを読めませんでした",
  "herdrImport.line.retry": "再試行",
  "herdrImport.line.refused": "移せなかった Herdr ワークスペース {{count}} 個",
  "herdrImport.line.refusedRow": "{{label}}：{{reason}}",
  "herdrImport.line.done": "OK",
  "herdrImport.line.waiting": "Herdr が閉じたので、{{count}} 件の会話をここで再開できます",
  "herdrImport.line.waitingAction": "再開",
  "herdrImport.import.title": "Herdr の作業を取り込む",
  "herdrImport.import.deviceTitle": "{{device}} の Herdr の作業を取り込む",
  "herdrImport.import.note": "タブ {{count}} 個が同じ配置とフォルダーで開きます。Herdr はそのままです。",
  "herdrImport.import.button": "取り込む",
} satisfies Catalog<typeof herdrImportEnglish>;

export const herdrImportCatalogs = { en: herdrImportEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof herdrImportEnglish>;
