// Desk's lane in this window (PRD session-docs-and-assistant D-64, B28-B35):
// which of its two columns show, what the tool column holds (the Threads
// list or one thread), the pin, and the tabs of the 보기 column. All of it is
// this window's and starts closed: the core keeps only the panels' widths
// (`desk.lane`, PRD desk-2-ui B23, B26).

import { create } from "zustand";
import type { DeskOutput } from "./snapshot";
export { namesView } from "./deskRoom";

/** A tab of the 보기 column: a job's output as its card listed it, or a mission's map (B34). */
export type DeskKeptTab = { kind: "output"; job: string; output: DeskOutput } | { kind: "mission"; mission: string; title: string };

/** Tabs the 보기 column keeps; keeping one more lets the oldest go. */
export const DESK_KEPT_TABS_MAX = 16;

/** What the tool column holds: the Threads list, or one thread, with the Conductor a worker's thread steps back to (B34). */
export type DeskTool = { kind: "threads" } | { kind: "thread"; job: string; conductor: string | null };

export type DeskLaneWindow = {
  /** The lane shows; ⌘E and Escape take it down and ⌘E brings back the columns it had. */
  up: boolean;
  /** The tool column's content, null while it is closed. */
  tool: DeskTool | null;
  /** The 보기 column shows. */
  view: boolean;
  /** The lane docks beside the chat rather than over it (B32). */
  pinned: boolean;
  /** The tabs kept in the 보기 column, oldest first, at most `DESK_KEPT_TABS_MAX`. */
  kept: DeskKeptTab[];
  /** The italic tab the next output pressed replaces (B31). */
  preview: DeskKeptTab | null;
  /** The key of the tab in front. */
  active: string | null;
  expanded: boolean;
  /** The thread `답하기` opened, and how many times, so its input box takes the keyboard once per press (B29). */
  answer: { job: string; seq: number } | null;
};

export const DESK_LANE_CLOSED: DeskLaneWindow = { up: false, tool: null, view: false, pinned: false, kept: [], preview: null, active: null, expanded: false, answer: null };

/** A tab's identity: one tab per output of a job, one per mission. */
export function tabKey(tab: DeskKeptTab): string {
  return tab.kind === "output" ? `output:${tab.job}:${tab.output.id}` : `mission:${tab.mission}`;
}

export function outputTab(job: string, output: DeskOutput): DeskKeptTab {
  return { kind: "output", job, output };
}

/** The 보기 column's tabs: the kept ones, then the preview unless it is one of them. */
export function viewTabs(kept: readonly DeskKeptTab[], lane: DeskLaneWindow): { tab: DeskKeptTab; preview: boolean }[] {
  const tabs = kept.map((tab) => ({ tab, preview: false }));
  if (lane.preview && !kept.some((tab) => tabKey(tab) === tabKey(lane.preview!))) tabs.push({ tab: lane.preview, preview: true });
  return tabs;
}

/** The tab in front: the one chosen, else the last. */
export function activeTab(kept: readonly DeskKeptTab[], lane: DeskLaneWindow): DeskKeptTab | null {
  const tabs = viewTabs(kept, lane);
  return tabs.find(({ tab }) => tabKey(tab) === lane.active)?.tab ?? tabs.at(-1)?.tab ?? null;
}

/** A column opened while the lane is down shows alone; with the lane up the other column stays (as the project lane's). */
function raise(lane: DeskLaneWindow, column: "tool" | "view"): DeskLaneWindow {
  if (lane.up) return lane;
  return { ...lane, up: true, tool: column === "tool" ? lane.tool : null, view: column === "view" };
}

/** A lane whose last column closed is down. */
function settle(lane: DeskLaneWindow): DeskLaneWindow {
  return lane.tool === null && !lane.view ? { ...lane, up: false, expanded: false } : lane;
}

/** The Threads list in the tool column (B28): ☰, and the empty Desk's `스레드 보기`. */
export function showThreads(lane: DeskLaneWindow): DeskLaneWindow {
  return { ...raise(lane, "tool"), tool: { kind: "threads" } };
}

/** ☰ (B28): the Threads list, or the tool column closed when it already shows. */
export function toggleThreads(lane: DeskLaneWindow): DeskLaneWindow {
  if (lane.up && lane.tool !== null) return closeTool(lane);
  return showThreads(lane);
}

/** ▣ (B31): the 보기 column, or it closed when it already shows. */
export function toggleOutputs(lane: DeskLaneWindow): DeskLaneWindow {
  if (lane.up && lane.view) return closeView(lane);
  return { ...raise(lane, "view"), view: true };
}

/** A card, a Threads row or a chip opens the thread in the tool column; `답하기` also puts the keyboard in its box (B25, B29). */
export function openThread(lane: DeskLaneWindow, job: string, options: { answer?: boolean; conductor?: string | null } = {}): DeskLaneWindow {
  return { ...raise(lane, "tool"), tool: { kind: "thread", job, conductor: options.conductor ?? null }, answer: options.answer ? { job, seq: (lane.answer?.seq ?? 0) + 1 } : null };
}

/** `←` from a thread: the Threads list, or the Conductor's thread a worker's was opened from (B34). */
export function backFromThread(lane: DeskLaneWindow): DeskLaneWindow {
  if (lane.tool?.kind !== "thread") return lane;
  return { ...lane, tool: lane.tool.conductor ? { kind: "thread", job: lane.tool.conductor, conductor: null } : { kind: "threads" } };
}

/**
 * An output or a mission's map pressed (B31, B34): a kept tab comes to the
 * front; any other replaces the preview. The caller keeps it in the core on
 * a second press.
 */
export function showTab(lane: DeskLaneWindow, kept: readonly DeskKeptTab[], tab: DeskKeptTab): DeskLaneWindow {
  const key = tabKey(tab);
  const isKept = kept.some((one) => tabKey(one) === key);
  return { ...raise(lane, "view"), view: true, active: key, preview: isKept ? lane.preview : tab };
}

/** A tab kept (a double click or ↵ on the preview) stops being the preview; past the cap the oldest kept tab goes. */
export function keepTab(lane: DeskLaneWindow, tab: DeskKeptTab): DeskLaneWindow {
  const key = tabKey(tab);
  const kept = lane.kept.some((one) => tabKey(one) === key) ? lane.kept : [...lane.kept, tab].slice(-DESK_KEPT_TABS_MAX);
  return lane.preview && tabKey(lane.preview) === key ? { ...lane, kept, preview: null, active: key } : { ...lane, kept };
}

/** A mission chip (B34): its map in the 보기 column and its Conductor's thread beside it. */
export function openMission(lane: DeskLaneWindow, kept: readonly DeskKeptTab[], mission: { id: string; title: string }, conductor: string | null): DeskLaneWindow {
  const shown = showTab({ ...lane, up: true, view: true }, kept, { kind: "mission", mission: mission.id, title: mission.title });
  return conductor ? { ...shown, tool: { kind: "thread", job: conductor, conductor: null } } : shown;
}

/** A tab's `✕`: the preview or the kept tab goes; the column closes with its last tab. */
export function closeTab(lane: DeskLaneWindow, kept: readonly DeskKeptTab[], tab: DeskKeptTab): DeskLaneWindow {
  const key = tabKey(tab);
  const left = viewTabs(kept.filter((one) => tabKey(one) !== key), { ...lane, preview: lane.preview && tabKey(lane.preview) === key ? null : lane.preview });
  const next = { ...lane, kept: lane.kept.filter((one) => tabKey(one) !== key), preview: lane.preview && tabKey(lane.preview) === key ? null : lane.preview, active: lane.active === key ? (left.at(-1) ? tabKey(left.at(-1)!.tab) : null) : lane.active };
  return left.length === 0 ? closeView(next) : next;
}

export function closeTool(lane: DeskLaneWindow): DeskLaneWindow {
  return settle({ ...lane, tool: null });
}

export function closeView(lane: DeskLaneWindow): DeskLaneWindow {
  return settle({ ...lane, view: false });
}

/** ⌘E and Escape on an unpinned lane: down whole, its columns kept for ⌘E. */
export function closeLane(lane: DeskLaneWindow): DeskLaneWindow {
  return { ...lane, up: false, expanded: false };
}

/** ⌘E: down while it shows; otherwise back with the columns it had, or the Threads list when it had none. */
export function toggleLane(lane: DeskLaneWindow): DeskLaneWindow {
  if (lane.up) return closeLane(lane);
  if (lane.tool === null && !lane.view) return { ...lane, up: true, tool: { kind: "threads" } };
  return { ...lane, up: true };
}

/** The lane's window state until the room replaces it. */
export const useDeskLaneStore = create<{ deskLane: DeskLaneWindow; setDeskLane: (change: (lane: DeskLaneWindow) => DeskLaneWindow) => void }>((set) => ({
  deskLane: DESK_LANE_CLOSED,
  setDeskLane: (change) => set((state) => ({ deskLane: change(state.deskLane) })),
}));
