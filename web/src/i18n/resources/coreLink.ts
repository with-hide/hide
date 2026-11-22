// The words of a node's link to its core on another machine (PRD
// core-host-node-move B16, W2, W3): the strip above the node's window and the
// core's mark in the device list.
import type { Catalog, Catalogs } from "../schema";

export const coreLinkEnglish = {
  "coreLink.disconnectMenu": "Disconnect from the {{machine}} core…",
  "coreLink.disconnectTitle": "Disconnect from the core on {{machine}}?",
  "coreLink.disconnectDescription": "The core refuses this Mac's pane input, file reads and browser actions at once. Reconnecting allows them again.",
  "coreLink.disconnectGo": "Disconnect",
  "coreLink.strip.updating": "Updating the core on {{machine}} to this app's build · agents keep running",
  "coreLink.strip.unreachable": "Cannot reach the core on {{machine}} · reconnecting",
  "coreLink.strip.splitWaits": "Cannot reach the core on {{machine}} · migration waits until it answers",
  "coreLink.strip.disconnected": "Disconnected from the core on {{machine}}",
  "coreLink.strip.signIn": "Signing in to the core on {{machine}} needs you",
  "coreLink.strip.signingIn": "Signing in to the core on {{machine}}",
  "coreLink.strip.signInRefused": "The core on {{machine}} refused the sign-in",
  "coreLink.strip.hostKeyChanged": "The core on {{machine}} answered with a different server key · check it in a terminal",
  "coreLink.strip.hideUpdated": "Hide was updated after it started · quit Hide and open it again to connect to the core on {{machine}}",
  "coreLink.strip.stopped": "ssh could not connect to the core on {{machine}} · run ssh {{alias}} in a terminal to see why",
  "coreLink.reconnect": "Reconnect",
  "coreLink.split.notReached": "Could not migrate to the new connection · {{machine}} was not reached over SSH",
  "coreLink.split.refused": "Could not migrate to the new connection · Hide on {{machine}} refused",
  "coreLink.split.notConnected": "Could not migrate to the new connection · the new connection to {{machine}} could not be made",
  "coreLink.split.notSaved": "Could not migrate to the new connection · this computer could not save it",
} as const;

const ko = {
  "coreLink.disconnectMenu": "{{machine}} core와 연결 끊기…",
  "coreLink.disconnectTitle": "{{machine}} core와 연결을 끊을까요?",
  "coreLink.disconnectDescription": "core가 이 Mac의 터미널 입력, 파일 읽기, 브라우저 조작을 바로 막아요. 다시 연결하면 다시 허용돼요.",
  "coreLink.disconnectGo": "연결 끊기",
  "coreLink.strip.updating": "{{machine}}의 core를 이 앱의 빌드로 바꾸는 중 · 에이전트는 계속 실행돼요",
  "coreLink.strip.unreachable": "{{machine}}의 core에 연결할 수 없어요 · 다시 연결하는 중",
  "coreLink.strip.splitWaits": "{{machine}}의 core에 연결할 수 없어요 · 이전 대기 중",
  "coreLink.strip.disconnected": "{{machine}} core와 연결을 끊었어요",
  "coreLink.strip.signIn": "{{machine}} core에 연결하려면 로그인이 필요해요",
  "coreLink.strip.signingIn": "{{machine}} core에 로그인하는 중",
  "coreLink.strip.signInRefused": "{{machine}} core가 로그인을 거절했어요",
  "coreLink.strip.hostKeyChanged": "{{machine}} core의 서버 키가 바뀌었어요 · 터미널에서 확인해 주세요",
  "coreLink.strip.hideUpdated": "Hide가 실행된 뒤에 업데이트됐어요 · Hide를 종료하고 다시 열면 {{machine}} core에 연결돼요",
  "coreLink.strip.stopped": "ssh로 {{machine}} core에 연결하지 못했어요 · 터미널에서 ssh {{alias}} 명령으로 이유를 확인해 주세요",
  "coreLink.reconnect": "다시 연결",
  "coreLink.split.notReached": "새 연결로 이전하지 못했어요 · SSH로 {{machine}}에 연결하지 못했어요",
  "coreLink.split.refused": "새 연결로 이전하지 못했어요 · {{machine}}의 Hide가 거절했어요",
  "coreLink.split.notConnected": "새 연결로 이전하지 못했어요 · {{machine}}에 새 연결로 닿지 못했어요",
  "coreLink.split.notSaved": "새 연결로 이전하지 못했어요 · 이 컴퓨터에 저장하지 못했어요",
} satisfies Catalog<typeof coreLinkEnglish>;

const zhCN = {
  "coreLink.disconnectMenu": "断开与 {{machine}} core 的连接…",
  "coreLink.disconnectTitle": "断开与 {{machine}} 上 core 的连接？",
  "coreLink.disconnectDescription": "core 会立即拒绝此 Mac 的窗格输入、文件读取和浏览器操作。重新连接后恢复。",
  "coreLink.disconnectGo": "断开连接",
  "coreLink.strip.updating": "正在将 {{machine}} 上的 core 更新为此应用的构建 · 智能体继续运行",
  "coreLink.strip.unreachable": "无法连接 {{machine}} 上的 core · 正在重新连接",
  "coreLink.strip.splitWaits": "无法连接 {{machine}} 上的 core · 等待迁移",
  "coreLink.strip.disconnected": "已断开与 {{machine}} 上 core 的连接",
  "coreLink.strip.signIn": "需要登录才能连接 {{machine}} 上的 core",
  "coreLink.strip.signingIn": "正在登录 {{machine}} 上的 core",
  "coreLink.strip.signInRefused": "{{machine}} 上的 core 拒绝了登录",
  "coreLink.strip.hostKeyChanged": "{{machine}} 上的 core 返回了不同的服务器密钥 · 请在终端中确认",
  "coreLink.strip.hideUpdated": "Hide 启动后已更新 · 请退出 Hide 并重新打开以连接 {{machine}} 上的 core",
  "coreLink.strip.stopped": "ssh 无法连接 {{machine}} 上的 core · 在终端运行 ssh {{alias}} 查看原因",
  "coreLink.reconnect": "重新连接",
  "coreLink.split.notReached": "未能迁移到新连接 · 无法通过 SSH 连接 {{machine}}",
  "coreLink.split.refused": "未能迁移到新连接 · {{machine}} 上的 Hide 拒绝了",
  "coreLink.split.notConnected": "未能迁移到新连接 · 无法与 {{machine}} 建立新连接",
  "coreLink.split.notSaved": "未能迁移到新连接 · 无法保存到此电脑",
} satisfies Catalog<typeof coreLinkEnglish>;

const ja = {
  "coreLink.disconnectMenu": "{{machine}} の core との接続を切る…",
  "coreLink.disconnectTitle": "{{machine}} の core との接続を切りますか？",
  "coreLink.disconnectDescription": "core はこの Mac のペイン入力、ファイル読み取り、ブラウザ操作をすぐに拒否します。再接続すると再び許可されます。",
  "coreLink.disconnectGo": "接続を切る",
  "coreLink.strip.updating": "{{machine}} の core をこのアプリのビルドに更新中 · エージェントは動き続けます",
  "coreLink.strip.unreachable": "{{machine}} の core に接続できません · 再接続中",
  "coreLink.strip.splitWaits": "{{machine}} の core に接続できません · 移行待ち",
  "coreLink.strip.disconnected": "{{machine}} の core との接続を切りました",
  "coreLink.strip.signIn": "{{machine}} の core に接続するにはサインインが必要です",
  "coreLink.strip.signingIn": "{{machine}} の core にサインインしています",
  "coreLink.strip.signInRefused": "{{machine}} の core がサインインを拒否しました",
  "coreLink.strip.hostKeyChanged": "{{machine}} の core が異なるサーバー鍵を返しました · ターミナルで確認してください",
  "coreLink.strip.hideUpdated": "Hide は起動後に更新されました · Hide を終了してもう一度開くと {{machine}} の core に接続できます",
  "coreLink.strip.stopped": "ssh で {{machine}} の core に接続できません · ターミナルで ssh {{alias}} を実行して原因を確認してください",
  "coreLink.reconnect": "再接続",
  "coreLink.split.notReached": "新しい接続へ移行できませんでした · SSH で {{machine}} に接続できませんでした",
  "coreLink.split.refused": "新しい接続へ移行できませんでした · {{machine}} の Hide が拒否しました",
  "coreLink.split.notConnected": "新しい接続へ移行できませんでした · {{machine}} との新しい接続を作れませんでした",
  "coreLink.split.notSaved": "新しい接続へ移行できませんでした · このコンピュータに保存できませんでした",
} satisfies Catalog<typeof coreLinkEnglish>;

export const coreLinkCatalogs = { en: coreLinkEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof coreLinkEnglish>;
