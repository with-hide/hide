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
  "startPanel.newMenu": ["Choose what to open", "새 탭 고르기", "新しいタブの種類", "新标签页选项"],
  "startPanel.terminal": ["Terminal", "터미널", "ターミナル", "终端"],
  "startPanel.startAgent": ["Start an agent…", "에이전트 시작…", "エージェントを開始…", "启动智能体…"],
  // A start on another computer is that computer's own work, started from its own screen (PRD device-connect-iroh D-73).
  "startPanel.handOff.needsScreen": [
    "Work on {{device}} starts on its own screen, which this window cannot show. Start it in the desktop app.",
    "{{device}}의 일은 그 컴퓨터 화면에서 시작해요. 이 창에서는 그 화면을 열 수 없으니 데스크톱 앱에서 시작하세요.",
    "{{device}} の作業はそのコンピューターの画面で開始します。このウィンドウではその画面を開けないため、デスクトップアプリで開始してください。",
    "{{device}} 的工作在它自己的屏幕上启动。此窗口无法打开那个屏幕，请在桌面应用中启动。",
  ],
  "startPanel.handOff.notShown": [
    "{{device}}'s screen did not open, so nothing started.",
    "{{device}}의 화면이 열리지 않아 시작하지 않았어요.",
    "{{device}} の画面が開かなかったため、開始していません。",
    "{{device}} 的屏幕未打开，因此未启动。",
  ],
  "startPanel.handOff.late": [
    "{{device}} was not ready in time, so nothing started.",
    "{{device}}에서 준비가 제때 끝나지 않아 시작하지 않았어요.",
    "{{device}} の準備が間に合わなかったため、開始していません。",
    "{{device}} 未能及时就绪，因此未启动。",
  ],
  "startPanel.handOff.notAdded": [
    "{{device}} could not add this folder as its project, so nothing started.",
    "{{device}}에서 이 폴더를 프로젝트로 더하지 못해 시작하지 않았어요.",
    "{{device}} でこのフォルダーをプロジェクトに追加できなかったため、開始していません。",
    "{{device}} 无法将此文件夹添加为项目，因此未启动。",
  ],
  "startPanel.handOff.noAnswer": [
    "No answer from {{device}}. Check its screen before starting again.",
    "{{device}}에서 답이 없어요. 다시 시작하기 전에 그 컴퓨터 화면을 확인하세요.",
    "{{device}} から応答がありません。もう一度開始する前に、その画面を確認してください。",
    "{{device}} 没有响应。再次启动前请先查看它的屏幕。",
  ],
  "startPanel.handOff.unconfirmed": [
    "Hide lost track of this start on {{device}}. Check its screen, then start again if it is not there.",
    "{{device}}에서 이 시작이 어떻게 됐는지 알 수 없게 됐어요. 그 컴퓨터 화면을 확인한 뒤 다시 시작하세요.",
    "{{device}} でこの開始がどうなったか分からなくなりました。その画面を確認してから、もう一度開始してください。",
    "无法确认此启动在 {{device}} 上的结果。请先查看它的屏幕，再重新启动。",
  ],
  "startPanel.handOff.busy": [
    "{{device}} is still answering earlier starts, so nothing started.",
    "{{device}}에서 앞선 시작들이 아직 끝나지 않아 시작하지 않았어요.",
    "{{device}} で前の開始がまだ終わっていないため、開始していません。",
    "{{device}} 上之前的启动尚未结束，因此未启动。",
  ],
} as const;

export const startPanelEnglish = Object.fromEntries(Object.entries(words).map(([key, value]) => [key, value[0]])) as { [K in keyof typeof words]: string };
export const startPanelCatalogs = Object.fromEntries((["en", "ko", "ja", "zh-CN"] as const).map((language, i) => [language, Object.fromEntries(Object.entries(words).map(([key, value]) => [key, value[i]!]))])) as Catalogs<typeof startPanelEnglish>;
