import { create } from "zustand";
import type { AgentGroup } from "./agentLayout";
import type { ViewWorkspace } from "./viewLayout";

export const useGroupFeedback = create<{ request: { workspace: ViewWorkspace; id: string } | null }>(() => ({ request: null }));

export type GroupNameIntent = { workspace: ViewWorkspace; groupId: string | null; name: string; tabId?: string };
export const useGroupName = create<{ intent: GroupNameIntent | null; open: (intent: GroupNameIntent) => void; close: () => void }>((set) => ({
  intent: null, open: (intent) => set({ intent }), close: () => set({ intent: null }),
}));

/** These are form checks only. The core independently validates the submitted name. */
export function groupNameProblem(name: string, groups: readonly AgentGroup[], id: string | null): "groups.invalidName" | "groups.duplicateName" | "groups.limit" | null {
  const trimmed = name.trim();
  if ([...trimmed].length === 0 || [...trimmed].length > 80 || /[\p{Cc}\p{Zl}\p{Zp}]/u.test(name)) return "groups.invalidName";
  if (groups.some((group) => group.id !== id && group.name === trimmed)) return "groups.duplicateName";
  if (id === null && groups.filter((group) => group.id !== null).length >= 32) return "groups.limit";
  return null;
}

/** Sidebar folds are presentation only; they never select a group or a pane. */
export const groupFoldKey = (device: string, checkout: string, group: string | null) => JSON.stringify([device, checkout, group]);
export const useGroupFolds = create<{ closed: ReadonlySet<string>; toggle: (key: string) => void }>((set) => ({
  closed: new Set(),
  toggle: (key) => set(({ closed }) => {
    const next = new Set(closed);
    if (next.has(key)) next.delete(key);
    else { if (next.size >= 8192) next.delete(next.values().next().value!); next.add(key); }
    return { closed: next };
  }),
}));
