import type { Catalog, Catalogs } from "../schema";

// Source paths, branches, session titles, snippets and conversation text are caller data.

export const sessionsEnglish = {
  "sessions.copyFailed": "Could not copy",
} as const;

const ko = {
  "sessions.copyFailed": "복사하지 못했어요",
} satisfies Catalog<typeof sessionsEnglish>;

const zhCN = {
  "sessions.copyFailed": "未能复制",
} satisfies Catalog<typeof sessionsEnglish>;

const ja = {
  "sessions.copyFailed": "コピーできませんでした",
} satisfies Catalog<typeof sessionsEnglish>;

export const sessionsCatalogs = { en: sessionsEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof sessionsEnglish>;
