// The words for why the pane menu's Sleep agent or Fork is drawn disabled: the
// core sends `sleep_action.reason` and `fork.reason` as codes
// (docs/LOCALIZATION.md, Translation boundary) and its English stays in the
// log. `PaneRelations.tsx` picks one of these by code, and a code missing here
// shows the item's general unavailable line.
import type { Catalogs } from "../schema";

export const agentActionReasonsEnglish = {
  "agentActionRefusal.device": "Agents on another device cannot sleep",
  "agentActionRefusal.unsupported": "This agent cannot sleep",
  "agentActionRefusal.core_machine_only": "This agent sleeps only on the device Hide's core runs on",
  "agentActionRefusal.asleep": "This agent is already asleep",
  "agentActionRefusal.working": "This agent is working",
  "agentActionRefusal.waiting_for_you": "This agent is waiting for you",
  "agentActionRefusal.unknown_activity": "Hide cannot tell what this agent is doing",
  "agentActionRefusal.no_conversation": "Herdr has not reported this agent's conversation yet",
} as const;

const ko = {
  "agentActionRefusal.device": "다른 기기의 에이전트는 재울 수 없어요",
  "agentActionRefusal.unsupported": "이 에이전트는 재울 수 없어요",
  "agentActionRefusal.core_machine_only": "이 에이전트는 core가 실행되는 기기에서만 재울 수 있어요",
  "agentActionRefusal.asleep": "이미 자고 있어요",
  "agentActionRefusal.working": "작업 중이에요",
  "agentActionRefusal.waiting_for_you": "내 답을 기다리는 중이에요",
  "agentActionRefusal.unknown_activity": "하는 일을 알 수 없어요",
  "agentActionRefusal.no_conversation": "Herdr가 이 에이전트의 대화를 아직 알려 주지 않았어요",
} satisfies Record<keyof typeof agentActionReasonsEnglish, string>;

const zhCN = {
  "agentActionRefusal.device": "其他设备上的智能体无法休眠",
  "agentActionRefusal.unsupported": "此智能体无法休眠",
  "agentActionRefusal.core_machine_only": "此智能体只能在运行 Hide core 的设备上休眠",
  "agentActionRefusal.asleep": "此智能体已在休眠",
  "agentActionRefusal.working": "此智能体正在工作",
  "agentActionRefusal.waiting_for_you": "此智能体正在等你回复",
  "agentActionRefusal.unknown_activity": "Hide 无法判断此智能体在做什么",
  "agentActionRefusal.no_conversation": "Herdr 尚未报告此智能体的对话",
} satisfies Record<keyof typeof agentActionReasonsEnglish, string>;

const ja = {
  "agentActionRefusal.device": "別のデバイスのエージェントは休止できません",
  "agentActionRefusal.unsupported": "このエージェントは休止できません",
  "agentActionRefusal.core_machine_only": "このエージェントは Hide の core が動くデバイスでのみ休止できます",
  "agentActionRefusal.asleep": "すでに休止中です",
  "agentActionRefusal.working": "作業中です",
  "agentActionRefusal.waiting_for_you": "あなたの返答を待っています",
  "agentActionRefusal.unknown_activity": "このエージェントが何をしているかわかりません",
  "agentActionRefusal.no_conversation": "Herdr がこのエージェントの会話をまだ報告していません",
} satisfies Record<keyof typeof agentActionReasonsEnglish, string>;

export const agentActionReasonsCatalogs = { en: agentActionReasonsEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof agentActionReasonsEnglish>;
