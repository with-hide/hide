// The start panel's own page state (PRD home-device-rail D-17, B30): whether
// it is open, the text the operator wrote, the target they picked while it is
// open, and the request in flight. The text outlives a close, so Esc keeps
// the draft until the next ⌘N and a start clears it; an agent that then fails
// to start puts the text it was given back (B31). The target is not kept:
// every open reads what is in front again (D-19).

import { create } from "zustand";
import type { ProviderKind } from "./agentPicker";
import type { MessageKey } from "./i18n/catalogs";

/**
 * What an open names instead of what is in front: the project to aim at, as
 * the first-run wizard's new project (PRD first-run-onboarding D-15) or a
 * project row's `+` (PRD sidebar-titlebar-refresh D-04), and the agent kind
 * when the surface chose one, else the remembered one.
 */
export type StartPreset = { projectId: string; kind?: ProviderKind };

/** Why a start did not go: the core's own sentence, shown as sent, or a catalog key the panel words in the operator's language. */
export type StartFailure = { message: string } | { key: MessageKey };

/** The codes the core sends in place of a sentence, each worded in the operator's language. */
const FAILURE_KEYS = new Map<string, MessageKey>([["codex_capability_unknown", "workspace.codexCapabilityUnknown"]]);

/** The core's reason when it sent one, worded by key when it is a code, else the plain "did not start". */
export function startFailure(message: string | null | undefined): StartFailure {
  if (!message) return { key: "workspace.agentNotStarted" };
  const key = FAILURE_KEYS.get(message);
  return key ? { key } : { message };
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
