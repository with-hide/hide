// What the phone app shows, as one store the socket writes and the screens
// read (PRD mobile-companion D-08). The list survives a lost connection so it
// can stay dimmed under the unreachable line (B23); the detail keeps the
// agent it opened by id, never by name (D-17).

import { create } from "zustand";
import type { PressedAnswer } from "../missionAnswer";
import type { AgentGroup, AgentKey, Conversation, DetailTab, DetailView, LookRow, Notice, Notifications, PairPayload, PhoneAgent, PhoneDevice, PhoneMission, PhonePr, PhonePrFeedback, PhonePrProject, PushMode, Refusal, RowsState, ServerFrame, StartCatalog } from "./protocol";
import { keyTag, listedAgents, mergeConversation, sameKey } from "./protocol";
import { resultContent, type LookContent } from "./lookContent";
import { holds, prOf, type MergeConfirm } from "./prModel";
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

/** The bottom bar's places (PRD mobile-companion-ui B28). */
export type PhoneTab = "agents" | "missions" | "prs";

/** The open pull requests as the last `prs` frame carried them; kept across a lost connection, dimmed under the unreachable line (B45). */
export type PrsState = { projects: PhonePrProject[]; items: PhonePr[]; feedback: PhonePrFeedback | null; omitted: number };

/**
 * The hand-over sheet of one pull request (B44): the prompt and the agent choice stay until a start succeeds, so a refusal or a
 * lost connection never costs what the operator edited. `feedbackId` names the feedback read the prompt is filled from.
 */
export type PrDelegateSheet = {
  key: string;
  prompt: string;
  /** The operator typed in the prompt: a read arriving later no longer replaces it. */
  edited: boolean;
  choice: StartChoice;
  feedbackId: string | null;
  /** The request id waiting for hided's answer. */
  pending: string | null;
  error: Notice | null;
};

/** An answer sent from a mission's page that the core has not yet kept or refused; the page draws it as the desktop's banner does. */
export type MissionAnswerSent = { requestId: string; mission: string; pressed: PressedAnswer };

/**
 * The answer this mission's page waits on, if any. One rule for "a press of this mission is still out":
 * the guard in `answerMission` and the page's buttons read it, so an answer left over from another
 * mission's page never stops this one.
 */
export function answerSentFor(sent: MissionAnswerSent | null, mission: string): MissionAnswerSent | null {
  return sent?.mission === mission ? sent : null;
}

export type Rows = { state: RowsState; text: string; lines: number; more: boolean };

/** The agent's open Views, as the last `looks` frame carried them; `omitted` are the ones past hided's cap. */
export type Looks = { rows: LookRow[]; omitted: number };

/**
 * One View opened full screen (B13). `requestId` names the read in flight, so a late frame of an earlier read, or of one the
 * phone retried, is ignored; `shown` is set once the phone told hided it saw the content (D-15).
 */
export type Viewing = { row: LookRow; requestId: string; content: LookContent; shown: boolean; refreshing: boolean };

/**
 * `moreAskedAt` is the line count when older rows were last asked for, and
 * `olderAskedAt` the cursor older messages were last asked before, so one
 * pull asks once. `conversation` is null until hided says whether the pane
 * has one; `none` leaves the terminal alone on screen. `view` is the half
 * hided reads; `tab` is what is on screen, which is 볼 것 without hided
 * knowing. `looks` is null until hided lists the Views.
 */
export type Detail = {
  key: AgentKey;
  view: DetailView;
  tab: DetailTab;
  rows: Rows | null;
  moreAskedAt: number | null;
  conversation: Conversation | "none" | null;
  olderAskedAt: number | null;
  looks: Looks | null;
  look: Viewing | null;
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
  /** The paired Mac and the devices it reaches, as the last list carried them. */
  devices: PhoneDevice[];
  /**
   * What this phone's operator chose on the list, which the desktop's sidebar never sees (PRD mobile-companion-ui D-03): a parent's
   * children opened or closed against what the core opened by default, keyed by `device|pane`; and the folds that were opened, keyed
   * `group:<id>` or `children:<device|pane>`. Both keep only rows and groups the list still has.
   */
  childrenOpen: Record<string, boolean>;
  foldsOpen: Record<string, true>;
  detail: Detail | null;
  /** The bottom bar's place while no detail is open, and the mission whose page is open on the missions tab; an agent opened from a step returns to it. */
  tab: PhoneTab;
  missionOpen: string | null;
  /** Null until the first missions frame; kept across a lost connection, dimmed under the unreachable line (B45). */
  missions: PhoneMission[] | null;
  /** The record could not be read on the Mac. */
  missionsFailed: boolean;
  /** The folds this phone opened on the missions tab: `closed` for 끝남's `모두`, `done:<mission>` for a page's 끝남; only missions the record still has. */
  missionFolds: Record<string, true>;
  /** What was typed for each mission's decision, kept until its answer is accepted (B35). */
  missionDrafts: Record<string, string>;
  missionAnswer: MissionAnswerSent | null;
  missionAnswerError: { mission: string; notice: Notice } | null;
  /** Null until the first `prs` frame; kept across a lost connection, dimmed under the unreachable line (B45). */
  prs: PrsState | null;
  /** The pull request whose page is open on the PR tab, by `PhonePr.key`. */
  prOpen: string | null;
  /** The merge confirmation, open on a page; it closes without sending when the page no longer shows what it named. */
  prConfirm: MergeConfirm | null;
  /** A merge sent from a page that hided has not answered yet. */
  prPending: { requestId: string; key: string } | null;
  /** The line under a page's buttons for a read or merge hided refused or the connection lost. */
  prNotice: { key: string; notice: Notice } | null;
  prDelegate: PrDelegateSheet | null;
  /** The pull request just handed to an agent, whose page says so until it is left. */
  prDelegated: string | null;
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
  devices: [],
  childrenOpen: {},
  foldsOpen: {},
  detail: null,
  tab: "agents",
  missionOpen: null,
  missions: null,
  missionsFailed: false,
  missionFolds: {},
  missionDrafts: {},
  missionAnswer: null,
  missionAnswerError: null,
  prs: null,
  prOpen: null,
  prConfirm: null,
  prPending: null,
  prNotice: null,
  prDelegate: null,
  prDelegated: null,
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
      patch({
        groups: frame.groups,
        devices: frame.devices,
        ...keptChoices(state, frame.groups),
        interfaceLanguage: frame.interface_language,
        colorTheme: frame.color_theme,
      });
      return;
    case "missions": {
      const listed = new Set(frame.items.map((mission) => mission.id));
      patch({
        missions: frame.items,
        missionsFailed: frame.failed,
        // A mission that left the record takes its page and its draft with it.
        missionOpen: state.missionOpen && listed.has(state.missionOpen) ? state.missionOpen : null,
        missionDrafts: Object.fromEntries(Object.entries(state.missionDrafts).filter(([id]) => listed.has(id))),
        missionFolds: Object.fromEntries(Object.entries(state.missionFolds).filter(([key]) => !key.startsWith("done:") || listed.has(key.slice("done:".length)))),
      });
      return;
    }
    case "prs": {
      const items = frame.items;
      const listed = new Set(items.map((pr) => pr.key));
      patch({
        prs: { projects: frame.projects, items, feedback: frame.feedback, omitted: frame.omitted ?? 0 },
        // A pull request that left the list takes its page, its confirmation and its sheet with it.
        prOpen: state.prOpen && listed.has(state.prOpen) ? state.prOpen : null,
        prConfirm: state.prConfirm && holds(state.prConfirm, prOf(items, state.prConfirm.key)) ? state.prConfirm : null,
        prDelegate: state.prDelegate && listed.has(state.prDelegate.key) ? state.prDelegate : null,
        prNotice: state.prNotice && listed.has(state.prNotice.key) ? state.prNotice : null,
        prDelegated: state.prDelegated && listed.has(state.prDelegated) ? state.prDelegated : null,
      });
      return;
    }
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
    case "looks": {
      const detail = state.detail;
      if (!detail || !sameKey(detail.key, frame)) return;
      patch({ detail: { ...detail, looks: { rows: frame.views, omitted: frame.omitted } } });
      return;
    }
    case "look_result":
      showLook(frame.request_id, resultContent(frame.state, frame.url));
      return;
    case "look_changes":
      showLook(frame.request_id, { state: "changes", files: frame.files, count: frame.count, added: frame.added, removed: frame.removed });
      return;
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

/** Puts `content` on the View the phone is looking at, when it is the answer to the read in flight. */
export function showLook(requestId: string, content: LookContent): void {
  const detail = usePhone.getState().detail;
  if (!detail?.look || detail.look.requestId !== requestId) return;
  patch({ detail: { ...detail, look: { ...detail.look, content, refreshing: false } } });
}

/** The choices this phone made that still name a row or group of the new list, so a long session keeps no stale ones. */
function keptChoices(state: PhoneState, groups: AgentGroup[]): Pick<PhoneState, "childrenOpen" | "foldsOpen"> {
  const rows = new Set(listedAgents(groups).map((agent) => keyTag(agent)));
  const folds = new Set([...groups.map((group) => foldKey.group(group.group)), ...[...rows].map(foldKey.children)]);
  return {
    childrenOpen: Object.fromEntries(Object.entries(state.childrenOpen).filter(([tag]) => rows.has(tag))),
    foldsOpen: Object.fromEntries(Object.entries(state.foldsOpen).filter(([key]) => folds.has(key))),
  };
}

export const foldKey = {
  group: (id: string) => `group:${id}`,
  children: (tag: string) => `children:${tag}`,
};

/** Opens or closes a parent's children on this phone only. */
export function setChildrenOpen(agent: PhoneAgent, open: boolean): void {
  usePhone.setState((state) => ({ childrenOpen: { ...state.childrenOpen, [keyTag(agent)]: open } }));
}

/** Opens a fold in place: a group's or a parent's "더 보기". */
export function openFold(key: string): void {
  usePhone.setState((state) => ({ foldsOpen: { ...state.foldsOpen, [key]: true } }));
}

export function setTab(tab: PhoneTab): void {
  usePhone.setState({ tab, missionOpen: null, prOpen: null });
}

/** Opens a mission's page on the missions tab. */
export function openMission(id: string): void {
  usePhone.setState({ tab: "missions", missionOpen: id, missionAnswerError: null });
}

export function closeMission(): void {
  usePhone.setState({ missionOpen: null, missionAnswerError: null });
}

/** Opens or closes a fold on the missions tab, on this phone only. */
export function setMissionFold(key: string, open: boolean): void {
  usePhone.setState((state) => {
    const folds = { ...state.missionFolds };
    if (open) folds[key] = true;
    else delete folds[key];
    return { missionFolds: folds };
  });
}

export function setMissionDraft(id: string, text: string): void {
  usePhone.setState((state) => ({ missionDrafts: { ...state.missionDrafts, [id]: text }, missionAnswerError: null }));
}

/** Opens a pull request's page on the PR tab. */
export function openPr(key: string): void {
  usePhone.setState({ tab: "prs", prOpen: key, prConfirm: null, prNotice: null, prDelegated: null });
}

export function closePr(): void {
  usePhone.setState({ prOpen: null, prConfirm: null, prNotice: null, prDelegated: null });
}

/** The mission a page shows, from the current record; null once it left the list. */
export function missionOf(missions: readonly PhoneMission[] | null, id: string | null): PhoneMission | null {
  return missions?.find((mission) => mission.id === id) ?? null;
}

/** The agent a detail shows, from the current list; null once it left the list. */
export function agentOf(groups: AgentGroup[] | null, key: AgentKey | null): PhoneAgent | null {
  if (!groups || !key) return null;
  return listedAgents(groups).find((candidate) => sameKey(candidate, key)) ?? null;
}
