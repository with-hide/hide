// What the phone app shows, as one store the socket writes and the screens
// read (PRD mobile-companion D-08). The list survives a lost connection so it
// can stay dimmed under the unreachable line (B23); the detail keeps the
// agent it opened by id, never by name (D-17).

import { create } from "zustand";
import type { AgentGroup, AgentKey, Conversation, DetailView, Notice, Notifications, PairPayload, PushMode, Refusal, RowsState, ServerFrame, StartCatalog } from "./protocol";
import { mergeConversation, sameKey } from "./protocol";
import { NO_CHOICE, type StartChoice } from "./start";

export type Screen =
  /** No code and no credential on this phone (B13). */
  | "version"
  | "unpaired"
  /** The QR opened this page: "Connect to <Mac>" and Connect (B11). */
  | "pair"
  /** Paired, and the list or a detail is showing. */
  | "app"
  /** hided refused this phone for good: an expired code, the phone limit, a revoke. */
  | "refused";

export type Rows = { state: RowsState; text: string; lines: number; more: boolean };

/**
 * `moreAskedAt` is the line count when older rows were last asked for, and
 * `olderAskedAt` the cursor older messages were last asked before, so one
 * pull asks once. `conversation` is null until hided says whether the pane
 * has one; `none` leaves the terminal alone on screen.
 */
export type Detail = {
  key: AgentKey;
  view: DetailView;
  rows: Rows | null;
  moreAskedAt: number | null;
  conversation: Conversation | "none" | null;
  olderAskedAt: number | null;
};

/**
 * The start sheet (B42-B44). The text and choice stay until a start succeeds,
 * so a refusal or a lost connection never costs the operator what they typed.
 */
export type StartSheet = {
  open: boolean;
  text: string;
  choice: StartChoice;
  /** The request id waiting for hided's answer. */
  pending: string | null;
  error: Notice | null;
};

export const CLOSED_SHEET: StartSheet = { open: false, text: "", choice: NO_CHOICE, pending: null, error: null };

export type PendingInput = { requestId: string; kind: "text" | "key" };

type PhoneState = {
  screen: Screen;
  relayError: "storage" | "busy" | "qrError" | "loadError" | null;
  deviceProtocol: number | null;
  pair: PairPayload | null;
  refusal: Refusal | null;
  /** A live, authenticated socket. */
  connected: boolean;
  /** At least one attempt failed and the socket is retrying (B23). */
  unreachable: boolean;
  /** Empty when the page address names no Mac. */
  macName: string;
  otherPhones: number;
  /** The core's explicit language as the last `agents` frame carried it; null before the first frame and while unset. */
  interfaceLanguage: string | null;
  /** The operator's color theme as the last `agents` frame carried it; null before the first frame (drawn as Hide). */
  colorTheme: string | null;
  pushMode: PushMode;
  notifications: Notifications;
  vapidKey: string | null;
  /** Null until the first list arrives; kept across a lost connection. */
  groups: AgentGroup[] | null;
  detail: Detail | null;
  /** The deep link waiting for the first list (B32). */
  pendingOpen: AgentKey | null;
  pendingInput: PendingInput | null;
  inputError: Notice | null;
  /** One request or failure at a time, keyed by the stable conversation intent. */
  sleepAction: { sleepId: string; requestId: string | null; error: boolean } | null;
  /** Shown once after pairing: Share › Add to Home Screen (B11). */
  installHint: boolean;
  /** What the start sheet lists; null until hided sends it after the sheet opens. */
  startCatalog: StartCatalog | null;
  startSheet: StartSheet;
  /** The agent a start just made, waiting to appear in the list before its detail opens. */
  startedAgent: AgentKey | null;
};

const initial: PhoneState = {
  screen: "unpaired",
  relayError: null,
  deviceProtocol: null,
  pair: null,
  refusal: null,
  connected: false,
  unreachable: false,
  macName: "",
  otherPhones: 0,
  interfaceLanguage: null,
  colorTheme: null,
  pushMode: "off",
  notifications: "unasked",
  vapidKey: null,
  groups: null,
  detail: null,
  pendingOpen: null,
  pendingInput: null,
  inputError: null,
  sleepAction: null,
  installHint: false,
  startCatalog: null,
  startSheet: CLOSED_SHEET,
  startedAgent: null,
};

export const usePhone = create<PhoneState>(() => initial);

export function patch(next: Partial<PhoneState>): void {
  usePhone.setState(next);
}

/** Applies one frame from hided; the socket owns what the frame asks it to do next. */
export function applyFrame(frame: ServerFrame): void {
  const state = usePhone.getState();
  switch (frame.type) {
    case "paired":
      patch({ screen: "app", pair: null, installHint: true });
      return;
    case "hello":
      patch({
        screen: "app",
        connected: true,
        unreachable: false,
        refusal: null,
        macName: frame.mac_name ?? "",
        vapidKey: frame.vapid_public_key,
        notifications: frame.notifications,
      });
      return;
    case "meta":
      patch({ pushMode: frame.push_mode, otherPhones: frame.other_phones });
      return;
    case "agents":
      patch({ groups: frame.groups, interfaceLanguage: frame.interface_language, colorTheme: frame.color_theme });
      return;
    case "sleep_result":
      if (state.sleepAction?.requestId === frame.request_id && state.sleepAction?.sleepId === frame.sleep_id) {
        patch({ sleepAction: { sleepId: frame.sleep_id, requestId: null, error: !frame.ok } });
      }
      return;
    case "rows": {
      const detail = state.detail;
      if (!detail || !sameKey(detail.key, frame)) return;
      const rows: Rows =
        frame.state === "ok"
          ? { state: "ok", text: frame.text ?? "", lines: frame.lines ?? 0, more: frame.more ?? false }
          : { state: frame.state, text: detail.rows?.text ?? "", lines: detail.rows?.lines ?? 0, more: false };
      patch({ detail: { ...detail, rows } });
      return;
    }
    case "conversation": {
      const detail = state.detail;
      if (!detail || !sameKey(detail.key, frame)) return;
      const current = detail.conversation === "none" ? null : detail.conversation;
      const conversation = frame.state === "ok" ? mergeConversation(current, frame) : "none";
      patch({ detail: { ...detail, conversation } });
      return;
    }
    case "start_catalog":
      patch({ startCatalog: { targets: frame.targets, core_platform: frame.core_platform, kinds: frame.kinds, remembered: frame.remembered } });
      return;
    case "push_state":
      patch({ notifications: frame.notifications === "on" ? "on" : state.notifications });
      return;
    default:
      return;
  }
}

/** The agent a detail shows, from the current list; null once it left the list. */
export function agentOf(groups: AgentGroup[] | null, key: AgentKey | null) {
  if (!groups || !key) return null;
  for (const group of groups) {
    const agent = group.agents.find((candidate) => sameKey(candidate, key));
    if (agent) return agent;
  }
  return null;
}
