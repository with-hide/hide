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
  // A spawn whose agent would start without Hide's hook on its machine
  // (`runtime::start_hook`); that machine's row in Settings says why.
  ["agent_hook_off", "coreError.startReason.agent_hook_not_ready"],
  ["agent_not_set_up", "coreError.startReason.agent_hook_not_ready"],
  ["agent_hook_untrusted", "coreError.startReason.agent_hook_not_ready"],
  ["agent_hook_missing", "coreError.startReason.agent_hook_not_ready"],
  ["agent_hook_unchecked", "coreError.startReason.agent_hook_not_ready"],
  ["agent_hook_unconfirmed", "coreError.startReason.agent_hook_unconfirmed"],
  // A start whose agent profile its machine did not answer for
  // (`hide-core/src/launch.rs`); the profile's name stays in the log.
  ["profile_gone", "coreError.startReason.profile_gone"],
  ["profile_refused", "coreError.startReason.profile_refused"],
  ["profiles_unsupported", "coreError.startReason.profiles_unsupported"],
  ["profile_unanswered", "coreError.startReason.profile_unanswered"],
  ["profile_unconfirmed", "coreError.startReason.profile_unconfirmed"],
  // Why waking a sleeping agent failed (`agent_sleep_engine::WAKE_*`).
  ["agent_wake.folder_missing", "taskReason.folderMissing"],
  ["agent_wake.not_resumed", "wakeReason.notResumed"],
  ["agent_wake.not_started", "wakeReason.notStarted"],
  ["agent_wake.not_ready", "wakeReason.notReady"],
  ["agent_wake.launch_choice_refused", "wakeReason.launchChoiceRefused"],
  // A task that failed for a reason the core words only in its log.
  ["task_operation.failed", "workspace.agentNotStarted"],
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
