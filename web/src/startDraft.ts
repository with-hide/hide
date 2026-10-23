// The start panel's own page state (PRD home-device-rail D-17, B30): whether
// it is open, the text the operator wrote, the target they picked while it is
// open, and the request in flight. The text outlives a close, so Esc keeps
// the draft until the next ⌘N and a start clears it; an agent that then fails
// to start puts the text it was given back (B31). The target is not kept:
// every open reads what is in front again (D-19).

import { create } from "zustand";
import type { ProviderKind } from "./agentPicker";
import type { MessageKey } from "./i18n/catalogs";
import { coreErrorKey } from "./coreError";
import { startReason } from "./startReason";
import type { AgentSelection } from "./agentPicker";
import type { AgentStartTarget } from "./agentLayout";

/**
 * What an open names instead of what is in front: the project to aim at, as
 * the first-run wizard's new project (PRD first-run-onboarding D-15) or a
 * project row's `+` (PRD sidebar-titlebar-refresh D-04), and the agent kind
 * when the surface chose one, else the remembered one.
 */
export type StartPreset = { projectId: string; kind?: ProviderKind };

/**
 * Why a start did not go: the core's own sentence, shown as sent, or a catalog
 * key the panel words in the operator's language, with the core's `code` for
 * the log and the element's `data-core-error` when it sent one.
 */
/** `line` is a pane screen's last line, quoted beside the sentence as the terminal showed it. */
export type StartFailure = { message: string } | { key: MessageKey; code?: string; line?: string };

/** The core's reason when it sent one (`startReason`), else the plain "did not start". */
/** A failed task's `failure` code worded (`startReason`); a code this build does not word is the plain "did not start", the code kept for `data-core-error`. */
export function startFailure(code: string | null | undefined): StartFailure {
  const worded = code ? startReason(code) : null;
  return worded && "key" in worded ? worded : { key: "workspace.agentNotStarted", ...(code ? { code } : {}) };
}

/**
 * Why a task's agent did not start, from `agent_reason`, always a code, and
 * `agent_line`; a code this build does not word, a daemon of another build's
 * included, is the plain "did not start", and stays on `data-core-error`.
 */
export function agentFailure(reason: string | null, line: string | null): StartFailure {
  const worded = reason ? startReason(reason) : null;
  const key = worded && "key" in worded ? worded.key : "workspace.agentNotStarted";
  const code = worded && "key" in worded ? worded.code : reason;
  return { key, ...(code ? { code } : {}), ...(line ? { line } : {}) };
}

/** A start the core refused, by its `last_error.kind`; a kind this build does not word is the plain "did not start". */
export function startRefusal(kind: string): StartFailure {
  return { key: coreErrorKey(kind) ?? "workspace.agentNotStarted", code: kind };
}

/** The core's reason as a sentence that another sentence can follow: a reason sent without its full stop gets one. */
export function endedSentence(text: string): string {
  const trimmed = text.trimEnd();
  return /[.!?。！？]$/.test(trimmed) ? trimmed : `${trimmed}.`;
}

type StartPanelState = {
  isOpen: boolean;
  text: string;
  /** The target picked in this open; null follows what is in front. */
  target: string | null;
  /** This open replaced Settings, which then counts as what is in front (B25). */
  overSettings: boolean;
  /** The project and agent this open starts on; null reads what is in front and the remembered agent. */
  preset: StartPreset | null;
  /** The id of the request whose answer the panel waits for. */
  request: string | null;
  requestKind: "agent_start" | "worktree_create";
  /** Why the last start did not go, shown inside the panel. */
  failure: StartFailure | null;
  /** The text the request in flight carries. */
  sent: string;
  /** A start whose tab is open but whose agent has not answered yet, with its text. */
  spent: { taskId: number; text: string } | null;
  open: (overSettings: boolean, preset?: StartPreset | null) => void;
  close: () => void;
  setText: (text: string) => void;
  setTarget: (key: string) => void;
  begin: (requestId: string, kind?: "agent_start" | "worktree_create") => void;
  fail: (failure: StartFailure) => void;
  /** The start went: the text is spent and the panel is done; `taskId` names an agent still starting. */
  finish: (taskId: number | null) => void;
  /** The spent start's agent did not start: its text comes back unless a new draft took its place. */
  restore: (failure: StartFailure) => void;
  /** The spent start's agent answered; its text is no longer kept. */
  settle: () => void;
};

export const useStartPanel = create<StartPanelState>((set) => ({
  isOpen: false,
  text: "",
  target: null,
  overSettings: false,
  preset: null,
  request: null,
  requestKind: "agent_start",
  failure: null,
  sent: "",
  spent: null,
  // A reason the last start did not go stays with its text until the text changes.
  open: (overSettings, preset = null) => set({ isOpen: true, target: null, overSettings, preset }),
  close: () => set({ isOpen: false }),
  setText: (text) => set({ text, failure: null }),
  setTarget: (target) => set({ target }),
  begin: (request, requestKind = "agent_start") => set((s) => ({ request, requestKind, failure: null, sent: s.text })),
  fail: (failure) => set({ request: null, failure }),
  finish: (taskId) => set((s) => ({ isOpen: false, text: "", request: null, failure: null, spent: taskId === null ? null : { taskId, text: s.sent } })),
  restore: (failure) => set((s) => (s.spent ? { text: s.text === "" ? s.spent.text : s.text, failure, spent: null } : {})),
  settle: () => set({ spent: null }),
}));

/** Input follows a core target through navigation; no group or tab topology lives here. */
export type InlineStartDraft = {
  text: string;
  selection: AgentSelection;
  request: null | { id: string; sentAt: number; provider: AgentSelection["kind"]; model: string | null; prompt: string | null; target: AgentStartTarget; restored?: boolean; beforeAnswer?: string };
  phase: "editing" | "pending" | "unknown" | "failed" | "agent_failed";
  taskId: number | null;
  failure: StartFailure | null;
};
const INLINE_DRAFT_LIMIT = 256;
// A real draft/live tab keeps its page input when core membership changes.
// Only an empty group's form needs group+area identity before a tab exists.
export const inlineStartKey = (device: string, path: string, target: AgentStartTarget) => JSON.stringify([device, path, target.tab_id === null ? ["area", target.group_id] : "tab", target.tab_id ?? target.area_id, target.add_pane]);
export const useInlineStarts = create<{
  drafts: Readonly<Record<string, InlineStartDraft>>;
  closing: Readonly<Record<string, { device: string; path: string; tab: string }>>;
  retired: ReadonlySet<string>;
  retireFailed: (requestId: string) => void;
  retainRetired: (requestIds: ReadonlySet<string>) => void;
  noteClose: (request: string, device: string, path: string, tab: string) => void;
  settleClose: (request: string, confirmed: boolean) => void;
  put: (key: string, draft: InlineStartDraft) => boolean;
  retarget: (device: string, path: string, requestId: string, target: AgentStartTarget) => boolean;
  drop: (key: string) => void;
}>((set, get) => ({
  drafts: {},
  closing: {},
  retired: new Set(),
  // These are dismissed answers, never topology. Keep them only while the core
  // still publishes those bounded receipts, so an old failure cannot rehydrate.
  retainRetired: (requestIds) => {
    const retired = get().retired;
    if ([...retired].some((id) => !requestIds.has(id))) set({ retired: new Set([...retired].filter((id) => requestIds.has(id))) });
  },
  retireFailed: (requestId) => {
    const state = get();
    if (state.retired.has(requestId)) return;
    const matches = Object.entries(state.drafts).filter(([, draft]) => draft.request?.id === requestId);
    if (matches.some(([, draft]) => draft.phase !== "failed")) return;
    const drafts = { ...state.drafts };
    for (const [key, draft] of matches) drafts[key] = { ...draft, request: null, phase: "editing", taskId: null, failure: null };
    set({ drafts, retired: new Set([...state.retired, requestId]) });
  },
  noteClose: (request, device, path, tab) => set((state) => ({ closing: { ...state.closing, [request]: { device, path, tab } } })),
  settleClose: (request, confirmed) => set((state) => {
    const closed = state.closing[request];
    if (!closed) return state;
    const closing = { ...state.closing }; delete closing[request];
    const drafts = { ...state.drafts };
    if (confirmed) for (const key of Object.keys(drafts)) {
      const [device, path, , tab] = JSON.parse(key) as string[];
      if (device === closed.device && path === closed.path && tab === closed.tab) delete drafts[key];
    }
    return { closing, drafts };
  }),
  // Reservation changes an empty-area address into a core draft identity. Move
  // its input in one update, without consuming another slot or minting an intent.
  retarget: (device, path, requestId, target) => {
    const key = inlineStartKey(device, path, target);
    const matches = Object.entries(get().drafts).filter(([heldKey, draft]) => {
      const [heldDevice, heldPath] = JSON.parse(heldKey) as string[];
      return heldDevice === device && heldPath === path && draft.request?.id === requestId;
    });
    const source = matches.find(([, draft]) => !draft.request?.restored) ?? matches[0];
    if (!source) return false;
    const [oldKey, draft] = source;
    const occupied = get().drafts[key];
    if (occupied && occupied.request?.id !== requestId) return false;
    const request = draft.request!;
    if (oldKey === key && matches.length === 1 && request.target.group_id === target.group_id && request.target.area_id === target.area_id && request.target.tab_id === target.tab_id && request.target.add_pane === target.add_pane) return true;
    set((state) => {
      const drafts = { ...state.drafts };
      for (const [heldKey] of matches) delete drafts[heldKey];
      drafts[key] = { ...draft, request: { ...request, target } };
      return { drafts };
    });
    return true;
  },
  put: (key, draft) => {
    if (!get().drafts[key] && Object.keys(get().drafts).length >= INLINE_DRAFT_LIMIT) return false;
    set((state) => ({ drafts: { ...state.drafts, [key]: draft } }));
    return true;
  },
  drop: (key) => set((state) => { const drafts = { ...state.drafts }; delete drafts[key]; return { drafts }; }),
}));
