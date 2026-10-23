import type { Catalog, Catalogs } from "../schema";

// Request and reply bodies, sender names, task titles and link labels are caller data.

export const requestsEnglish = {
  "requests.images_one": "{{count}} image",
  "requests.images_other": "{{count}} images",
  "requests.sender.operator": "Me",
  "requests.sender.agent": "Agent",
  "requests.end.working": "Working",
  "requests.end.question": "Question",
  "requests.end.blocked": "Blocked",
  "requests.end.done": "Done",
  "requests.end.waiting": "Waiting",
  "requests.end.unfinished": "Unfinished",
  "requests.end.limited": "Usage limit",
  "requests.verdict": "AI assessment · {{end}}",
  "requests.verdictLine": "AI assessment · {{end}} · {{line}}",
  "requests.children_one": "{{count}} descendant",
  "requests.children_other": "{{count}} descendants",
  "requests.workingChildren": "Working {{count}}",
  "requests.badge.open": "Open",
} as const;

const ko = {
  "requests.images_one": "이미지 {{count}}",
  "requests.images_other": "이미지 {{count}}",
  "requests.sender.operator": "나",
  "requests.sender.agent": "에이전트",
  "requests.end.working": "작업 중",
  "requests.end.question": "질문",
  "requests.end.blocked": "막힘",
  "requests.end.done": "끝남",
  "requests.end.waiting": "대기",
  "requests.end.unfinished": "일 남음",
  "requests.end.limited": "한도",
  "requests.verdict": "AI 판정 · {{end}}",
  "requests.verdictLine": "AI 판정 · {{end}} · {{line}}",
  "requests.children_one": "자식 {{count}}",
  "requests.children_other": "자식 {{count}}",
  "requests.workingChildren": "작업 중 {{count}}",
  "requests.badge.open": "열림",
} satisfies Catalog<typeof requestsEnglish>;

const zhCN = {
  "requests.images_one": "{{count}} 张图片",
  "requests.images_other": "{{count}} 张图片",
  "requests.sender.operator": "我",
  "requests.sender.agent": "智能体",
  "requests.end.working": "进行中",
  "requests.end.question": "提问",
  "requests.end.blocked": "受阻",
  "requests.end.done": "已完成",
  "requests.end.waiting": "等待中",
  "requests.end.unfinished": "未完成",
  "requests.end.limited": "额度用尽",
  "requests.verdict": "AI 判断 · {{end}}",
  "requests.verdictLine": "AI 判断 · {{end}} · {{line}}",
  "requests.children_one": "{{count}} 个子智能体",
  "requests.children_other": "{{count}} 个子智能体",
  "requests.workingChildren": "工作中 {{count}}",
  "requests.badge.open": "打开",
} satisfies Catalog<typeof requestsEnglish>;

const ja = {
  "requests.images_one": "画像{{count}}枚",
  "requests.images_other": "画像{{count}}枚",
  "requests.sender.operator": "自分",
  "requests.sender.agent": "エージェント",
  "requests.end.working": "進行中",
  "requests.end.question": "質問",
  "requests.end.blocked": "ブロック",
  "requests.end.done": "完了",
  "requests.end.waiting": "待機",
  "requests.end.unfinished": "未完了",
  "requests.end.limited": "上限到達",
  "requests.verdict": "AI判定 · {{end}}",
  "requests.verdictLine": "AI判定 · {{end}} · {{line}}",
  "requests.children_one": "子エージェント{{count}}件",
  "requests.children_other": "子エージェント{{count}}件",
  "requests.workingChildren": "作業中 {{count}}",
  "requests.badge.open": "オープン",
} satisfies Catalog<typeof requestsEnglish>;

export const requestsCatalogs = { en: requestsEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof requestsEnglish>;
