// The one wording of a reason the core gives for an agent start it refused.
// The core sends a code in place of a sentence where the operator's language
// matters; every surface that shows such a reason (the start panel, the task
// line, a session resume, a failed wake) words it here.

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";

/** The codes the core sends for a start it refused, each worded once in the catalog. */
const REASON_KEYS = new Map<string, MessageKey>([["codex_capability_unknown", "agents.codexUnread"]]);

/** A start's reason as the screen words it: the catalog's sentence for a code, else the core's own text as sent. */
export function startReason(text: string): { key: MessageKey } | { message: string } {
  const key = REASON_KEYS.get(text);
  return key ? { key } : { message: text };
}

/** `startReason` as the text a surface shows. */
export function startReasonText(text: string, t: TFunction<"translation">): string {
  const reason = startReason(text);
  return "key" in reason ? t(reason.key) : reason.message;
}
