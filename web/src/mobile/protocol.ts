// The phone's side of `/ws` (PRD mobile-companion D-01, D-04, D-16, D-17,
// D-22): the frames hided sends a phone (hided/src/mobile/phone.rs), the
// ones a phone may send back, how the page address carries a pairing code,
// a credential or a deep link, and the words each refusal and pane state
// shows. Pure, so every rule is tested without a socket or a browser.
// Text is named by catalog key and translated where it renders, so a shown
// line follows a language change.

import type { TFunction } from "i18next";
import type { MessageKey } from "../i18n/catalogs";
import type { AgentSleep, AgentStatusCode, AgentWait, LineMode, RowMark, SleepingSession } from "../snapshot";

export type PhoneAgent = {
  device_id: string;
  pane_id: string;
  root_pane_id: string;
  group: GroupId;
  /** The desktop row's mark (`RowMark`). */
  mark: RowMark;
  /** What an in-progress row waits on, which its line words when it has no sentence. */
  wait: AgentWait | null;
  /** How many of its descendants work, for a row waiting on them. */
  descendants_working: number;
  /** The background tasks a finished turn left running (status-marks D-19); absent when none. */
  background_left?: number;
  agent_kind: string;
  title: string;
  place: string | null;
  device_label: string | null;
  /** When the Mac's core last saw this agent change state (epoch ms); the phone counts the elapsed time from it. */
  changed_at_unix_ms: number | null;
  /** The desktop row's second line (`RowLine`), worded the same way. */
  line: { text: string; mode: LineMode; tone: "warning" | "news" | "subtle"; pull_request?: number } | null;
  status_code: AgentStatusCode;
  demand: string;
  /** Present while the agent's process is ended and its pane holds a bare shell: nothing may be sent to it, and the detail says so (`AgentSleepSnapshot`). */
  sleep?: AgentSleep;
  emphasized: boolean;
  holds_notification: boolean;
};

export type GroupId = "needs_you" | "done" | "working" | "seen";

/** A conversation intent has no pane address, lineage or filesystem path. */
export type PhoneSleepingSession = {
  sleep_id: string;
  device_id: string;
  agent_kind: string;
  title: string;
  group: GroupId;
  phase: SleepingSession["phase"];
  since_unix_ms: number;
  wake_available: boolean;
  checking: boolean;
  attention: boolean;
};

export type AgentGroup = { group: GroupId; count: number; agents: PhoneAgent[]; sleeping_sessions?: PhoneSleepingSession[] };

export type Notifications = "on" | "off" | "unasked";

export type PushMode = "off" | "app_closed" | "always";

export type RowsState = "ok" | "gone" | "device_unreachable" | "unavailable";

/** Which half of a detail the phone shows. */
export type DetailView = "conversation" | "terminal";

/** One message of an agent's conversation (hided/src/mobile/conversation.rs). */
export type ConversationMessage = {
  /** The byte offset of its transcript record: unique and in order. */
  id: number;
  who: "you" | "agent" | "stopped";
  text: string;
  truncated: boolean;
  at_ms: number;
};

/** The agent kinds a phone may start (hided/src/mobile/start.rs KINDS). */
export type StartKind = string;

/** One place a start can go; the folder it names never reaches the phone. */
export type StartTarget = {
  /** `home:<device_id>` for a device's Home, else the checkout's id. */
  id: string;
  device_id: string;
  /** The place on the device; null is the device's Home. */
  place: string | null;
  /** The device's name; null is the core's own machine, named from the catalog's `core_platform`. */
  device_label: string | null;
  connected: boolean;
};

export type StartKindEntry = { id: StartKind; models: string[] };

/** What the start sheet lists, as hided sends it while the sheet is open. */
export type StartCatalog = {
  targets: StartTarget[];
  /** The core's machine's system as its device row says it (`macos aarch64`), from which the phone says This Mac, This PC or This computer. */
  core_platform: string | null;
  kinds: StartKindEntry[];
  /** The kind and each kind's model the last start chose, shared with the desktop. */
  remembered: { kind: StartKind | null; models: Partial<Record<StartKind, string>> };
};

export type ServerFrame =
  | { type: "paired"; credential: string; phone_id: string; name: string }
  | { type: "hello"; mac_name: string | null; phone_id: string; name: string | null; vapid_public_key: string; notifications: Notifications }
  | { type: "meta"; push_mode: PushMode; other_phones: number }
  /**
   * `interface_language` is the core's explicit choice as stored (en, ko, zh-CN, ja), or null while the phone follows its own language.
   * `color_theme` is the operator's palette (`hide`, `neutral`); the phone keeps its own light or dark.
   */
  | { type: "agents"; groups: AgentGroup[]; interface_language: string | null; color_theme: string | null }
  | { type: "rows"; device_id: string; pane_id: string; state: RowsState; text?: string; lines?: number; more?: boolean }
  | {
      type: "conversation";
      device_id: string;
      pane_id: string;
      /** `none`: the pane has no conversation to show, so its terminal stands alone. */
      state: "ok" | "none";
      mode?: "reset" | "append" | "older";
      messages?: ConversationMessage[];
      /** The cursor for the page before the oldest message; null at the start. */
      before?: number | null;
    }
  | { type: "input_result"; request_id?: string; ok: boolean; reason: string | null }
  /** Accepted for dispatch; progress and refusal remain on the core-owned row. A pane's wake answers with `sleepTarget` of its key as `sleep_id`. */
  | { type: "sleep_result"; request_id?: string; sleep_id: string; ok: boolean; reason: string | null }
  | ({ type: "start_catalog" } & StartCatalog)
  | { type: "start_result"; request_id?: string; ok: boolean; reason: string | null; device_id?: string; pane_id?: string }
  | { type: "push_state"; notifications: "on" | "refused" }
  /** The answer to `ping`: hided is there. */
  | { type: "pong"; id: number }
  | { type: "refused"; reason: string }
  | { type: "refused_request"; request: string; reason: string };

/** The only messages a phone sends after its first frame. */
export type PhoneMessage =
  | { type: "open"; device_id: string; pane_id: string; view: DetailView }
  | { type: "view"; view: DetailView }
  | { type: "older"; before: number }
  | { type: "more" }
  | { type: "close" }
  | { type: "wake_sleeping_session" | "check_sleeping_session"; request_id: string; sleep_id: string }
  | { type: "wake_agent"; request_id: string; device_id: string; pane_id: string; fresh?: boolean }
  | ({ type: "input"; request_id: string; device_id: string; pane_id: string } & ({ text: string } | { key: QuickKey }))
  | { type: "start_sheet"; open: boolean }
  | { type: "start_agent"; request_id: string; text: string; target: string; kind: StartKind; model?: string }
  | { type: "push_subscription"; subscription: { endpoint: string; keys: { p256dh: string; auth: string } } }
  | { type: "push_permission"; state: "denied" | "default" }
  /** Asks whether hided still answers; the phone gives up the socket when nothing comes back. */
  | { type: "ping"; id: number };

export type AgentKey = { device_id: string; pane_id: string };

/** The target a pane's wake is answered under (`AgentKey::tag` in hided): one id per pane, as a SleepId is per conversation. */
export function sleepTarget(key: AgentKey): string {
  return `${key.device_id}|${key.pane_id}`;
}

export function sameKey(a: AgentKey | null, b: AgentKey | null): boolean {
  return !!a && !!b && a.device_id === b.device_id && a.pane_id === b.pane_id;
}

/** The sentences that carry a count limit, named by `{{limit}}`. */
type LimitKey = "mobile.input.tooLong" | "mobile.start.tooLong" | "mobile.refusal.phoneLimit";

/** The other input, refusal, row and start sentences carry no value. */
type PlainKey = Exclude<Extract<MessageKey, `mobile.input.${string}` | `mobile.refusal.${string}` | `mobile.rows.${string}` | `mobile.start.${string}`>, LimitKey>;

/** A sentence the phone shows, kept as its catalog key so it follows a language change. */
export type Notice = { key: PlainKey } | { key: LimitKey; limit: number };

export function noticeText(t: TFunction<"translation">, notice: Notice): string {
  return "limit" in notice ? t(notice.key, { limit: notice.limit }) : t(notice.key);
}

export const GROUP_TITLE: Record<GroupId, MessageKey> = {
  needs_you: "mobile.group.needs_you",
  done: "mobile.group.done",
  working: "mobile.group.working",
  seen: "mobile.group.seen",
};


/** The five quick keys (B26, B38): what hided sends Herdr, the keycap, and the accessible name. */
export type QuickKey = "enter" | "escape" | "up" | "down" | "ctrl_c";

export const QUICK_KEYS: readonly { key: QuickKey; label: string; name: MessageKey }[] = [
  { key: "enter", label: "⏎", name: "mobile.key.enter" },
  { key: "escape", label: "Esc", name: "mobile.key.escape" },
  { key: "up", label: "↑", name: "mobile.key.up" },
  { key: "down", label: "↓", name: "mobile.key.down" },
  { key: "ctrl_c", label: "^C", name: "mobile.key.ctrlC" },
];

/** The longest reply hided writes in one input (hided/src/mobile/pane.rs MAX_REPLY_CHARS). */
export const MAX_REPLY_CHARS = 2000;

/** Why a reply cannot be sent yet, checked before it leaves the phone (B26, B27). */
export function replyProblem(text: string): string | null {
  if (text.trim().length === 0) return "empty";
  if ([...text].length > MAX_REPLY_CHARS) return "too_long";
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f-\u009f]/.test(text)) return "control_characters";
  return null;
}

/** The line under the reply bar for a refused or failed input. */
export function inputFailure(reason: string | null): Notice {
  switch (reason) {
    case "uncertain":
      return { key: "mobile.input.uncertain" };
    case "in_flight":
      return { key: "mobile.input.inFlight" };
    case "empty":
      return { key: "mobile.input.empty" };
    case "too_long":
      return { key: "mobile.input.tooLong", limit: MAX_REPLY_CHARS };
    case "control_characters":
      return { key: "mobile.input.controlCharacters" };
    case "gone":
      return { key: "mobile.input.gone" };
    case "asleep":
      return { key: "mobile.input.asleep" };
    case "device_unreachable":
      return { key: "mobile.input.deviceUnreachable" };
    case "offline":
      return { key: "mobile.input.offline" };
    default:
      return { key: "mobile.input.failed" };
  }
}

/** A refused connection's screen (B13, B14, B16). */
export type Refusal = "code_expired" | "phone_limit" | "revoked" | "mobile_off" | "no_credential";

export function refusalOf(reason: string): Refusal {
  switch (reason) {
    case "code_expired":
    case "phone_limit":
    case "revoked":
    case "mobile_off":
      return reason;
    default:
      return "revoked";
  }
}

/** The phones hided pairs at once (hided/src/mobile/phones.rs MAX_PHONES). */
export const MAX_PHONES = 4;

export const REFUSAL_NOTICE: Record<Refusal, Notice> = {
  code_expired: { key: "mobile.refusal.codeExpired" },
  phone_limit: { key: "mobile.refusal.phoneLimit", limit: MAX_PHONES },
  revoked: { key: "mobile.refusal.revoked" },
  mobile_off: { key: "mobile.refusal.offline" },
  no_credential: { key: "mobile.refusal.noCredential" },
};

export const UNREACHABLE_NOTICE = REFUSAL_NOTICE.mobile_off;

/** The detail's line for a pane hided could not read (B28). */
export function rowsProblem(state: RowsState): Notice | null {
  switch (state) {
    case "ok":
      return null;
    case "gone":
      return { key: "mobile.input.gone" };
    case "device_unreachable":
      return { key: "mobile.rows.deviceUnreachable" };
    default:
      return { key: "mobile.rows.unavailable" };
  }
}

/**
 * A row drawn only with box-drawing characters, such as the rule a TUI draws
 * across the desktop pane: wrapped at a phone's width it becomes a stack of
 * rules, so the detail clips it to one line instead.
 */
export function boxDrawingRow(text: string): boolean {
  return /^[\s\u2500-\u257f]*[\u2500-\u257f][\s\u2500-\u257f]*$/.test(text);
}

/** The most messages a detail holds; at it, no older page is asked for. */
export const MAX_MESSAGES = 300;

export type Conversation = { messages: ConversationMessage[]; before: number | null };

/** What a detail holds after one conversation frame: a fresh page, appended messages, or an older page. */
export function mergeConversation(
  current: Conversation | null,
  frame: { mode?: "reset" | "append" | "older"; messages?: ConversationMessage[]; before?: number | null },
): Conversation {
  const incoming = frame.messages ?? [];
  if (!current || frame.mode === "reset" || !frame.mode) return { messages: incoming, before: frame.before ?? null };
  if (frame.mode === "older") {
    const first = current.messages[0]?.id ?? Number.POSITIVE_INFINITY;
    return { messages: [...incoming.filter((message) => message.id < first), ...current.messages], before: frame.before ?? null };
  }
  const last = current.messages.at(-1)?.id ?? -1;
  const messages = [...current.messages, ...incoming.filter((message) => message.id > last)];
  if (messages.length <= MAX_MESSAGES) return { messages, before: current.before };
  // Past the cap the oldest go, and the page before the new oldest is still there to read.
  const kept = messages.slice(-MAX_MESSAGES);
  return { messages: kept, before: kept[0]?.id ?? null };
}

/** A message's time: the hour and minute today, with the day before today. */
export function messageTime(atMs: number, now: Date = new Date()): string {
  const at = new Date(atMs);
  const time = `${String(at.getHours()).padStart(2, "0")}:${String(at.getMinutes()).padStart(2, "0")}`;
  const sameDay = at.getFullYear() === now.getFullYear() && at.getMonth() === now.getMonth() && at.getDate() === now.getDate();
  return sameDay ? time : `${at.getMonth() + 1}/${at.getDate()} ${time}`;
}

/** The QR's payload (D-22): `{v:1, endpoint, code}` as base64url JSON. */
export type PairPayload = { v: 1; endpoint: string; code: string };

function fromBase64Url(text: string): string {
  const base64 = text.replace(/-/g, "+").replace(/_/g, "/");
  const binary = atob(base64 + "=".repeat((4 - (base64.length % 4)) % 4));
  return new TextDecoder().decode(Uint8Array.from(binary, (char) => char.charCodeAt(0)));
}

export function toBase64Url(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function parsePair(value: string): PairPayload | null {
  try {
    const parsed: unknown = JSON.parse(fromBase64Url(value));
    if (typeof parsed !== "object" || parsed === null) return null;
    const { v, endpoint, code } = parsed as Record<string, unknown>;
    if (v !== 1 || typeof endpoint !== "string" || typeof code !== "string" || code.length === 0) return null;
    return { v: 1, endpoint, code };
  } catch {
    return null;
  }
}

/** What the page address carries after `#` (D-22): a pairing code, a credential, a deep link. */
export type Fragment = { pair: PairPayload | null; credential: string | null; open: AgentKey | null };

export function parseFragment(hash: string): Fragment {
  const params = new URLSearchParams(hash.replace(/^#/, ""));
  const pair = params.get("pair");
  const credential = params.get("k");
  const open = params.get("open");
  return {
    pair: pair ? parsePair(pair) : null,
    credential: credential && /^[0-9a-f]{64}$/.test(credential) ? credential : null,
    open: open ? openKey(open) : null,
  };
}

/** A deep link names one agent as `device|pane`, the notification tag's form (D-17). */
export function openKey(value: string): AgentKey | null {
  const bar = value.indexOf("|");
  if (bar <= 0 || bar === value.length - 1) return null;
  return { device_id: value.slice(0, bar), pane_id: value.slice(bar + 1) };
}

export function keyTag(key: AgentKey): string {
  return `${key.device_id}|${key.pane_id}`;
}

/**
 * The page address a paired phone keeps, so "Add to Home Screen" carries the
 * credential into the installed app, whose storage iOS may keep apart from
 * Safari's (PRD Risks).
 */
export function credentialHash(credential: string): string {
  return `#k=${credential}`;
}

/** The Mac's name before pairing: the first label of the ts.net address; empty when the address names none. */
export function macNameOf(endpoint: string): string {
  try {
    return new URL(endpoint).hostname.split(".")[0] ?? "";
  } catch {
    return "";
  }
}

/** The list header's second line (B19). */
export function headerLine(t: TFunction<"translation">, macName: string, otherPhones: number): string {
  return otherPhones > 0 ? t("mobile.otherPhones", { name: macName, count: otherPhones }) : macName;
}

/** Tags whose notifications the open app closes (D-21): agents no longer waiting or done. */
export function staleTags(tags: readonly string[], groups: readonly AgentGroup[]): string[] {
  const live = new Set<string>();
  for (const group of groups) {
    for (const agent of group.agents) {
      if (agent.holds_notification) live.add(keyTag({ device_id: agent.device_id, pane_id: agent.root_pane_id }));
    }
  }
  return tags.filter((tag) => !live.has(tag));
}

/** The notification row on the list (B30), or null when it has nothing to say. */
export type NotificationRow = "enable" | "install_first" | "denied" | null;

export function notificationRow({
  pushMode,
  notifications,
  permission,
  supported,
}: {
  pushMode: PushMode;
  notifications: Notifications;
  permission: NotificationPermission | "unsupported";
  /** Whether this page can subscribe at all: a Safari tab on iOS cannot, a Home Screen app can. */
  supported: boolean;
}): NotificationRow {
  if (pushMode === "off") return null;
  if (permission === "denied" || (notifications === "off" && permission !== "granted")) return "denied";
  if (notifications === "on" && permission === "granted") return null;
  // iOS offers push only to a Home Screen app; a Safari tab has no PushManager.
  if (!supported) return "install_first";
  return "enable";
}
