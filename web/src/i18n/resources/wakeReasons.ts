// The words for why waking a sleeping agent failed: the core sends an
// `agent_wake.*` code as the sleep's `reason`, and the folder and Herdr's own
// answer stay in the `agent_sleep.wake_failed` diagnostic (docs/LOCALIZATION.md,
// Translation boundary). `startReason.ts` maps each code to its key.
import type { Catalogs } from "../schema";

export const wakeReasonsEnglish = {
  "wakeReason.notResumed": "The conversation couldn't be resumed.",
  "wakeReason.notStarted": "A new session couldn't be started.",
  "wakeReason.notReady": "The agent did not become ready in time.",
  "wakeReason.launchChoiceRefused": "This agent no longer takes the model or effort it was started with.",
} as const;

const ko = {
  "wakeReason.notResumed": "대화를 이어 가지 못했어요.",
  "wakeReason.notStarted": "새 세션을 시작하지 못했어요.",
  "wakeReason.notReady": "에이전트가 제시간에 준비되지 않았어요.",
  "wakeReason.launchChoiceRefused": "이 에이전트는 시작할 때 고른 모델이나 effort 설정을 더 이상 받지 않아요.",
} satisfies Record<keyof typeof wakeReasonsEnglish, string>;

const zhCN = {
  "wakeReason.notResumed": "无法继续这段对话。",
  "wakeReason.notStarted": "无法开始新会话。",
  "wakeReason.notReady": "智能体未能及时就绪。",
  "wakeReason.launchChoiceRefused": "此智能体已不再接受启动时选择的模型或 effort。",
} satisfies Record<keyof typeof wakeReasonsEnglish, string>;

const ja = {
  "wakeReason.notResumed": "会話を再開できませんでした。",
  "wakeReason.notStarted": "新しいセッションを開始できませんでした。",
  "wakeReason.notReady": "エージェントが時間内に準備できませんでした。",
  "wakeReason.launchChoiceRefused": "このエージェントは、起動時に選んだモデルや effort をもう受け付けません。",
} satisfies Record<keyof typeof wakeReasonsEnglish, string>;

export const wakeReasonsCatalogs = { en: wakeReasonsEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof wakeReasonsEnglish>;
