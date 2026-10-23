// Chapter 1's world: one invented project with one Claude agent at work in
// its only tab, answering splits, focus, zoom, pane moves, tabs and groups,
// and closing a pane and reopening it the way the core's recent-close stack does.

import type { LayoutNode, RecentClosed } from "../../snapshot";
import type { CoreEvent, PracticeWorld } from "../world";
import { agent, applyDesk, deskSnapshot, oneTabDesk, panesOf, shellScreen, type DeskModel, type PracticePane, type PracticePlace, type PracticeTab } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };

/** A closed pane as the core keeps it to reopen: the pane and its agent, and the tab it stood in as it was. */
export type ClosedPane = { pane: PracticePane; tab: PracticeTab; group: string | null; position: number; order: number };

/** The desk, the closes it can reopen, newest last, and the pane the last reopen brought back with what it was. */
export type ScreenModel = DeskModel & { closed: ClosedPane[]; reopened: { paneId: string; tabId: string; from: ClosedPane } | null };

function initial(): ScreenModel {
  // A working agent asks before it is closed, as the core decides for one.
  const lead = agent({ pane_id: "lead", identity_label: "Fix the checkout 500", agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "1m", requires_close_confirmation: true });
  return { ...oneTabDesk({ id: "lead", agent: lead, title: "Fix the checkout 500" }), closed: [], reopened: null };
}

/** `node` without the pane `id`; a split left with one side becomes that side. */
function without(node: LayoutNode, id: string): LayoutNode | null {
  if (node.type === "pane") return node.pane_id === id ? null : node;
  const first = without(node.first, id);
  const second = without(node.second, id);
  if (!first) return second;
  if (!second) return first;
  return { ...node, first, second };
}

const rename = (node: LayoutNode, from: string, to: string): LayoutNode =>
  node.type === "pane" ? (node.pane_id === from ? { ...node, pane_id: to } : node) : { ...node, first: rename(node.first, from, to), second: rename(node.second, from, to) };

const insert = <T,>(rows: T[], index: number, row: T) => [...rows.slice(0, index), row, ...rows.slice(index)];

/**
 * The desk after the pane `id` closed. A working agent closes only once the
 * operator confirmed it on the Stop-work sheet; its tab's last pane closes the
 * tab, and the area then shows the tab before it in its group
 * (docs/UI_BEHAVIOR.md, closing a tab). The desk's last pane is not closed
 * here; the chapter never needs an empty desk.
 */
function closePane(model: ScreenModel, id: string, confirmed: boolean): ScreenModel | null {
  const pane = model.panes.find((row) => row.id === id);
  const tab = model.tabs.find((row) => panesOf(row.root).includes(id));
  if (!pane || !tab || model.panes.length === 1) return null;
  if (pane.agent?.requires_close_confirmation && !confirmed) return null;
  const group = model.groups.find((row) => row.tabs.includes(tab.id))!;
  const closed: ClosedPane = { pane, tab, group: group.id, position: group.tabs.indexOf(tab.id), order: model.tabs.indexOf(tab) };
  const panes = model.panes.filter((row) => row !== pane);
  const root = without(tab.root, id);
  if (root) {
    const focused = tab.focused === id ? panesOf(root)[0]! : tab.focused;
    return { ...model, panes, tabs: model.tabs.map((row) => (row === tab ? { ...row, root, focused, zoomed: false } : row)), closed: [...model.closed, closed] };
  }
  const tabs = model.tabs.filter((row) => row !== tab);
  const groups = model.groups.map((row) => ({ ...row, tabs: row.tabs.filter((one) => one !== tab.id) }));
  const siblings = group.tabs.filter((one) => one !== tab.id);
  const front = model.activeTab !== tab.id ? model.activeTab : (siblings[closed.position - 1] ?? siblings[closed.position] ?? tabs[0]!.id);
  return { ...model, panes, tabs, groups, activeTab: front, selectedGroup: groups.find((row) => row.tabs.includes(front))!.id, closed: [...model.closed, closed] };
}

/**
 * The newest close brought back in a new pane, as Herdr makes one, holding the
 * same agent continuing its conversation: in its tab where it stood when the
 * tab is as the close left it, beside the tab's focused pane when the tab
 * changed since, and as a tab of its own in its former group and place when
 * the tab closed with it (docs/UI_BEHAVIOR.md, Reopen closed tab).
 */
function reopen(model: ScreenModel): ScreenModel | null {
  const last = model.closed.at(-1);
  if (!last) return null;
  const paneId = `${last.pane.agent ? "ag" : "sh"}${model.nextId}`;
  const pane: PracticePane = { ...last.pane, id: paneId, agent: last.pane.agent && { ...last.pane.agent, pane_id: paneId, id: paneId } };
  const base = { ...model, panes: [...model.panes, pane], nextId: model.nextId + 1, closed: model.closed.slice(0, -1) };
  const tab = model.tabs.find((row) => row.id === last.tab.id);
  if (tab) {
    const left = panesOf(last.tab.root).filter((id) => id !== last.pane.id);
    const asLeft = panesOf(tab.root).join("\0") === left.join("\0");
    const root: LayoutNode = asLeft ? rename(last.tab.root, last.pane.id, paneId) : { type: "split", direction: "right", ratio: 0.5, first: tab.root, second: { type: "pane", pane_id: paneId } };
    const group = model.groups.find((row) => row.tabs.includes(tab.id))!;
    return { ...base, tabs: model.tabs.map((row) => (row === tab ? { ...row, root, focused: paneId, zoomed: false } : row)), activeTab: tab.id, selectedGroup: group.id, reopened: { paneId, tabId: tab.id, from: last } };
  }
  const reopened: PracticeTab = { ...last.tab, id: `t${base.nextId}`, root: { type: "pane", pane_id: paneId }, focused: paneId, zoomed: false };
  const home = model.groups.find((row) => row.id === last.group) ?? model.groups.find((row) => row.id === null)!;
  return {
    ...base, nextId: base.nextId + 1,
    tabs: insert(model.tabs, last.order, reopened),
    groups: model.groups.map((row) => (row === home ? { ...row, tabs: insert(row.tabs, row.id === last.group ? last.position : row.tabs.length, reopened.id) } : row)),
    activeTab: reopened.id, selectedGroup: home.id,
    reopened: { paneId, tabId: reopened.id, from: last },
  };
}

function apply(model: ScreenModel, event: CoreEvent): ScreenModel | null {
  switch (event.kind) {
    case "close_pane":
      return closePane(model, String(event.payload.pane_id), event.payload.confirmed === true);
    case "reopen_closed":
      return reopen(model);
    default: {
      const desk = applyDesk(model, event);
      return desk && { ...model, ...desk };
    }
  }
}

/** The core's recent-close stack as the snapshot carries it, which the shell's Reopen closed tab reads before it sends. */
function recentClosed(model: ScreenModel): RecentClosed {
  const top = model.closed.at(-1);
  return { count: model.closed.length, top_label: top?.pane.title ?? null, restoring: false, pending: [], can_reopen: top !== undefined, reopen_blocked_reason: null };
}

/** Whether the last reopen brought the closed pane back as it was: in its tab, with its agent and its title. */
export function reopenedAsClosed(model: ScreenModel): boolean {
  const back = model.reopened;
  const pane = back && model.panes.find((row) => row.id === back.paneId);
  const tab = back && model.tabs.find((row) => row.id === back.tabId);
  if (!back || !pane || !tab || !panesOf(tab.root).includes(pane.id)) return false;
  return pane.title === back.from.pane.title && pane.agent?.identity_label === back.from.pane.agent?.identity_label;
}

/** What each pane's terminal shows when it first appears; a reopened agent shows the conversation it continues. */
function paneScreen(pane: PracticePane): string {
  if (!pane.agent) return shellScreen(PLACE);
  return [
    "\x1b[38;5;209m✻\x1b[0m Claude Code",
    "",
    "\x1b[2m>\x1b[0m Checkout returns a 500 when I pay. Fix it",
    "",
    "\x1b[38;5;209m●\x1b[0m Following the payment request through the code.",
    "  \x1b[2m⎿  Reading src/payment/checkout.ts…\x1b[0m",
    "",
    "\x1b[38;5;209m✻\x1b[0m \x1b[2mLooking for the cause… (esc to interrupt)\x1b[0m",
  ].join("\r\n");
}

export const SCREEN_WORLD: PracticeWorld<ScreenModel> = {
  initial,
  apply,
  snapshot: (model, nowMs) => {
    const drawn = deskSnapshot([{ place: PLACE, model }], PLACE.project, nowMs);
    return { ...drawn, rest: { ...drawn.rest, recent_closed: recentClosed(model) } };
  },
  panes: (model) => model.panes.map((pane) => pane.id),
  screen: (model, paneId) => paneScreen(model.panes.find((pane) => pane.id === paneId)!),
  lane: false,
};
