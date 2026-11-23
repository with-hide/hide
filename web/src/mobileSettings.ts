// The pure rules behind Settings › Devices' phone group (PRD mobile-companion
// B1-B7, B10, B14, B15, B29; settings-cleanup B57-B59; operator review
// 2026-11-23): which Tailscale steps show and with what fix, how the code
// countdown and a phone's row read, when the group says why phones cannot
// connect, and the phone push setting. hided owns every value; this only
// words what its `mobile` frame reported, so each rule is tested without a browser.

import type { TFunction } from "i18next";
import { formatRelativeTime } from "./i18n/format";
import type { MessageKey } from "./i18n/catalogs";
import type { InterfaceLanguage } from "./i18n/locale";

export type StepState = "ok" | "failed" | "waiting";

/** The daemon's `mobile` frame (hided/src/mobile/mod.rs `frame_value`). */
export type MobileState = {
  enabled: boolean;
  method: "relay" | "tailscale";
  relay: { state: "off" | "connecting" | "connected" | "retrying" | "refused" | "key_unavailable"; reason?: string };
  exposure: "off" | "checking" | "blocked" | "foreign" | "failed" | "exposed";
  checklist: { installed: StepState; logged_in: StepState; https: StepState; host_name: string | null } | null;
  download_url: string;
  admin_url: string;
  foreign_target: string | null;
  failure: { step: string; message: string } | null;
  url: string | null;
  qr: string | null;
  code_expires_at_ms: number | null;
  phones: PhoneRow[];
  max_phones: number;
  push_mode: PushMode;
  now_ms: number;
};

export type PhoneRow = {
  id: string;
  /** The name it is shown under: the operator's choice, else the one it paired with. */
  name: string;
  /** The name it paired with, sent only once the operator renamed it; absent from a daemon older than renaming. */
  own_name?: string | null;
  last_seen_ms: number;
  connected: boolean;
  notifications: "on" | "off" | "unasked";
  revoke_at_ms: number;
};

export type PushMode = "app_closed" | "always";

/** A Tailscale step that needs the operator: what is wrong, what to do, and where to do it. */
export type FailedStep = {
  id: "installed" | "logged_in" | "https";
  title: string;
  action: string;
  link: { label: string; href: string } | null;
};

/**
 * What the Tailscale checks show (PRD settings-cleanup B57): one ready line
 * when all three Mac steps passed, only the failing steps with their fix when
 * some failed, and nothing while a step is still being read (the exposure
 * line says it is checking). The phone-side step is guidance hide cannot
 * check; it lives in the pairing row's description.
 */
export type ChecklistView = { kind: "ready" } | { kind: "failed"; steps: FailedStep[] } | { kind: "pending" };

/** `machine`: the core's machine as this window names it, which runs Tailscale for the phone (`coreMachineName`). */
export function checklistView(state: MobileState, machine: string, t: TFunction<"translation">): ChecklistView {
  const steps = state.checklist ?? { installed: "waiting", logged_in: "waiting", https: "waiting", host_name: null };
  const failed: FailedStep[] = [];
  if (steps.installed === "failed") {
    failed.push({ id: "installed", title: t("mobileSetup.installMissing", { machine }), action: t("mobileSetup.installAction"), link: { label: t("mobileSetup.download"), href: state.download_url } });
  }
  if (steps.logged_in === "failed") {
    failed.push({ id: "logged_in", title: t("mobileSetup.loginMissing"), action: t("mobileSetup.loginAction"), link: null });
  }
  if (steps.https === "failed") {
    failed.push({ id: "https", title: t("mobileSetup.httpsMissing"), action: t("mobileSetup.httpsAction"), link: { label: t("mobileSetup.adminConsole"), href: state.admin_url } });
  }
  if (failed.length > 0) return { kind: "failed", steps: failed };
  return steps.installed === "ok" && steps.logged_in === "ok" && steps.https === "ok" ? { kind: "ready" } : { kind: "pending" };
}

/** The one line beside the checks when hide itself has something to say about the serve entry, or null. */
export function exposureLine(state: MobileState, machine: string, t: TFunction<"translation">): { tone: "pending" | "warn" | "error"; text: string } | null {
  switch (state.exposure) {
    case "checking":
      return { tone: "pending", text: t("mobileSetup.checking") };
    case "foreign":
      return { tone: "warn", text: t("mobileSetup.foreign", { machine, target: state.foreign_target ?? t("mobileSetup.unknownTarget") }) };
    case "failed":
      return { tone: "error", text: t("mobileSetup.failed", { step: t(failureStep(state.failure?.step)), message: state.failure?.message ?? "" }).trim() };
    default:
      return null;
  }
}

function failureStep(step: string | undefined): MessageKey {
  switch (step) {
    case "add":
      return "mobileSetup.failure.add";
    case "remove":
      return "mobileSetup.failure.remove";
    case "funnel":
      return "mobileSetup.failure.funnel";
    default:
      return "mobileSetup.failure.inspect";
  }
}

/** `m:ss` until the code expires, from the daemon's clock; null once expired. */
export function codeCountdown(expiresAtMs: number | null, daemonNowMs: number, elapsedSinceFrameMs: number): string | null {
  if (expiresAtMs === null) return null;
  const left = Math.ceil((expiresAtMs - daemonNowMs - elapsedSinceFrameMs) / 1000);
  if (left <= 0) return null;
  return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`;
}

const DAY_MS = 24 * 60 * 60 * 1000;

/** "Just now" under a minute, then minutes, hours and days ago in the interface language. */
export function lastSeen(lastSeenMs: number, nowMs: number, connected: boolean, t: TFunction<"translation">, language: InterfaceLanguage): string {
  if (connected) return t("mobileSetup.justNow");
  const ago = Math.max(0, nowMs - lastSeenMs);
  if (ago < 60_000) return t("mobileSetup.justNow");
  if (ago < 3_600_000) return formatRelativeTime(language, -Math.floor(ago / 60_000), "minute");
  if (ago < DAY_MS) return formatRelativeTime(language, -Math.floor(ago / 3_600_000), "hour");
  return formatRelativeTime(language, -Math.floor(ago / DAY_MS), "day");
}

/**
 * A phone row's second line (B15): when it was last seen while it is not
 * connected (its row's state says connected otherwise), the notification
 * state when the phone answered the permission question, and the days until
 * the seven-day revoke once it has been away a day. Empty when there is
 * nothing to say.
 */
export function phoneLine(phone: PhoneRow, nowMs: number, t: TFunction<"translation">, language: InterfaceLanguage): string {
  const parts = phone.connected ? [] : [lastSeen(phone.last_seen_ms, nowMs, phone.connected, t, language)];
  if (phone.notifications === "on") parts.push(t("mobileSetup.notificationsOn"));
  if (phone.notifications === "off") parts.push(t("mobileSetup.notificationsOff"));
  const away = nowMs - phone.last_seen_ms;
  if (!phone.connected && away >= DAY_MS) {
    const days = Math.max(1, Math.ceil((phone.revoke_at_ms - nowMs) / DAY_MS));
    parts.push(t("mobileSetup.autoRevoke", { count: days }));
  }
  return parts.join(" · ");
}

/** What the group's one reason line offers: try the relay again, or update this Hide. */
export type PhoneProblemAction = "retry" | "update" | null;

/**
 * Why no phone can connect now, said once over the phone rows, or null
 * (operator review 2026-11-23): only while 폰에서 Hide 열기 is on (or a
 * failed removal is still owed) and the selected way is down or refused;
 * a relay still connecting, or a connection that works, says nothing.
 * `machine`: the core's machine as this window names it.
 */
export function phoneProblem(state: MobileState, machine: string, t: TFunction<"translation">): { text: string; action: PhoneProblemAction } | null {
  if (!state.enabled && state.exposure !== "failed") return null;
  if (state.method === "tailscale") {
    const line = exposureLine(state, machine, t);
    if (line && line.tone !== "pending") return { text: line.text, action: null };
    const view = checklistView(state, machine, t);
    return view.kind === "failed" ? { text: view.steps[0]!.title, action: null } : null;
  }
  const status = state.relay.state;
  if (state.failure) {
    return { text: status === "connected" ? t("mobileSetup.relay.cleanupFailed") : (exposureLine({ ...state, exposure: "failed" }, machine, t)?.text ?? t("mobileSetup.relay.refused")), action: null };
  }
  if (status === "retrying") return { text: t("mobileSetup.relay.retrying"), action: "retry" };
  if (status === "key_unavailable") return { text: t("mobileSetup.relay.keyUnavailable"), action: "retry" };
  if (status === "refused") {
    const outdated = state.relay.reason === "unsupported_version";
    return { text: t(RELAY_REFUSAL[state.relay.reason as keyof typeof RELAY_REFUSAL] ?? "mobileSetup.relay.refused"), action: outdated ? "update" : "retry" };
  }
  return null;
}

const RELAY_REFUSAL = {
  connection_limit: "mobileSetup.relay.connectionLimit",
  room_limit: "mobileSetup.relay.roomLimit",
  rate_limit: "mobileSetup.relay.rateLimit",
  frame_limit: "mobileSetup.relay.frameLimit",
  daily_limit: "mobileSetup.relay.dailyLimit",
  room_occupied: "mobileSetup.relay.occupied",
  unsupported_version: "mobileSetup.relay.outdated",
} as const satisfies Record<string, MessageKey>;
