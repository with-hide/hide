import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import type { HerdrProfileRefusal, KitUnavailable, KitWhy } from "./snapshot";

type Translate = TFunction<"translation">;

// The sentence for each cause; a cause whose sentence names a value has an `At` key for it.
const WHY_KEYS = new Map<string, { plain?: MessageKey; at?: MessageKey }>([
  ["taken_out", { plain: "kitWhy.taken_out" }],
  ["older_version", { plain: "kitWhy.older_version" }],
  ["edited", { plain: "kitWhy.edited" }],
  ["helper_gone", { at: "kitWhy.helper_goneAt" }],
  ["route_changed", { plain: "kitWhy.route_changed" }],
  ["other_build", { at: "kitWhy.other_buildAt" }],
  ["not_hides", { plain: "kitWhy.not_hides" }],
  ["no_helper", { at: "kitWhy.no_helperAt" }],
  ["unreadable", { plain: "kitWhy.unreadable", at: "kitWhy.unreadableAt" }],
  ["other_program", { at: "kitWhy.other_programAt" }],
  ["foreign", { at: "kitWhy.foreignAt" }],
  ["agent_absent", { plain: "kitWhy.agent_absent" }],
  ["not_created_yet", { plain: "kitWhy.not_created_yet", at: "kitWhy.not_created_yetAt" }],
  ["program_missing", { at: "kitWhy.program_missingAt" }],
  ["unsupported", { plain: "kitWhy.unsupported" }],
  ["update_agent", { at: "kitWhy.update_agentAt" }],
  ["ai_folder", { plain: "kitWhy.ai_folder" }],
  ["still_in_place", { plain: "kitWhy.still_in_place" }],
  ["not_changed", { plain: "kitWhy.not_changed" }],
  ["codex_untrusted", { plain: "kitWhy.codex_untrusted" }],
  ["record_unsaved", { plain: "kitWhy.record_unsaved" }],
  ["herdr_absent", { plain: "kitWhy.herdr_absent" }],
  ["update_herdr", { plain: "kitWhy.update_herdr" }],
  ["retirement_pending", { plain: "kitWhy.retirement_pending" }],
]);

/**
 * Why a kit part or an agent's piece is in its state, in the operator's language, or null when there is no cause or
 * this build does not know it (a newer helper's): the state then speaks alone. The kit's English sentence stays in
 * the log and the `hide` CLI.
 */
export function kitWhyText(why: KitWhy | null | undefined, t: Translate): string | null {
  if (!why) return null;
  const keys = WHY_KEYS.get(why.cause);
  if (!keys) return null;
  if (why.value && keys.at) return t(keys.at, { value: why.value });
  if (keys.plain) return t(keys.plain);
  // A cause whose sentence needs its value, sent without one.
  return null;
}

const UNAVAILABLE_KEYS = new Map<KitUnavailable, MessageKey>([
  ["standalone", "kitUnavailable.standalone"],
  ["no_consent", "kitUnavailable.no_consent"],
  ["consent_outdated", "kitUnavailable.consent_outdated"],
  ["unsupported_platform", "kitUnavailable.unsupported_platform"],
  ["installer_unavailable", "kitUnavailable.installer_unavailable"],
  ["unreadable", "kitUnavailable.unreadable"],
]);

const PROFILE_REFUSAL_KEYS = new Map<HerdrProfileRefusal, MessageKey>([
  ["no_consent", "herdrProfileRefusal.no_consent"],
  ["disabled", "herdrProfileRefusal.disabled"],
  ["invalid_target", "herdrProfileRefusal.invalid_target"],
  ["discovery_pending", "herdrProfileRefusal.discovery_pending"],
  ["device_limit", "herdrProfileRefusal.device_limit"],
  ["profile_moved", "herdrProfileRefusal.profile_moved"],
]);

/** Why Hide declined to connect a saved Herdr profile; a code this build does not know gets the general sentence. */
export function profileRefusalText(refusal: HerdrProfileRefusal, t: Translate): string {
  return t(PROFILE_REFUSAL_KEYS.get(refusal) ?? "herdrProfileRefusal.other");
}

/** Why a machine's kit does not run at all; a code this build does not know gets the general sentence. */
export function kitUnavailableText(cause: KitUnavailable, t: Translate): string {
  return t(UNAVAILABLE_KEYS.get(cause) ?? "kitUnavailable.other");
}
