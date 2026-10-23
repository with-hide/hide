// The one wording of a reason the core gives for an agent start it refused.
// The core sends a code in place of a sentence where the operator's language
// matters; every surface that shows such a reason (the start panel, the task
// line, a session resume, a failed wake) words it here.

import type { TFunction } from "i18next";
import { coreErrorKey } from "./coreError";
import type { MessageKey } from "./i18n/catalogs";

/** The codes the core sends for a start it refused, each worded once in the catalog. */
const REASON_KEYS = new Map<string, MessageKey>([
  ["codex_capability_unknown", "agents.codexUnread"],
  ["session_route_unconfirmed", "coreError.startReason.session_route_unconfirmed"],
  ["session_route_reader_unavailable", "coreError.startReason.session_route_unconfirmed"],
  ["session_route_reference_unconfirmed", "coreError.startReason.session_route_unconfirmed"],
  ["session_route_cwd_unconfirmed", "coreError.startReason.session_route_unconfirmed"],
  ["session_route_in_motion", "coreError.startReason.session_route_in_motion"],
  ["session_resume_scope_required", "coreError.startReason.session_resume_scope_required"],
  ["session_resume_intent_changed", "coreError.startReason.session_resume_intent_changed"],
  ["session_resume_reader_changed", "coreError.startReason.session_resume_intent_changed"],
  ["session_resume_control_changed", "coreError.startReason.session_resume_control_changed"],
  ["cursor_launch_environment_invalid", "coreError.startReason.cursor_launch_environment_invalid"],
  ["cursor_launch_root_unsupported", "coreError.startReason.cursor_launch_root_unsupported"],
  ["cursor_launch_root_ambiguous", "coreError.startReason.cursor_launch_root_ambiguous"],
  ["cursor_launch_root_unconfirmed", "coreError.startReason.cursor_launch_root_unconfirmed"],
  ["cursor_launch_root_changed", "coreError.startReason.cursor_launch_root_changed"],
]);

/** A node's refusal of a native session file arrives as `session_checkpoint_invalid:<code>`; the code is what is worded. */
const CHECKPOINT_PREFIX = "session_checkpoint_invalid:";

/** A start's reason as the screen words it: the catalog's sentence for a code, else the core's own text as sent. */
export function startReason(text: string): { key: MessageKey; code: string } | { message: string } {
  const code = text.startsWith(CHECKPOINT_PREFIX) ? text.slice(CHECKPOINT_PREFIX.length) : text;
  // A start that failed after the core accepted it may carry one of its refusal kinds (`agent_start.*`).
  const key = REASON_KEYS.get(code) ?? coreErrorKey(code);
  return key ? { key, code } : { message: text };
}

/** `startReason` as the text a surface shows. */
export function startReasonText(text: string, t: TFunction<"translation">): string {
  const reason = startReason(text);
  return "key" in reason ? t(reason.key) : reason.message;
}
