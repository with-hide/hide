// The several-computers flow's world: This Mac with one agent at work, and a
// second computer (the Mac mini, reached over SSH) with two of its own, one
// at work and one asking, so its tile on the device rail carries a mark. The
// rail's tile brings that computer forward (`focus_device`); the screen then
// draws what the core projected from it (`status.remote[].session`): its
// projects, its agents, its tabs and panes, under the band that names it. A
// new tab there goes to it as a `remote_control` request, and opens a shell
// on that computer. Nothing here connects to anything: both machines are
// invented, and so is the invented path under /work.

import type { AgentScope, ProjectTasksScope } from "../../agentScope";
import type { AgentRow, Checkout, LayoutNode, PaneHeader, RemotePaneFrame, RemotePaneLayout, SnapshotRest, Workspace } from "../../snapshot";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, deskSnapshot, focusedPane, oneTabDesk, panesOf, rowRequest, rowState, shellScreen, type DeskModel, type PracticePane, type PracticePlace } from "./kit";

export const MINI = "mini";
const HERE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };
const THERE: PracticePlace = { project: "billing-api", label: "billing-api", branch: "main" };
const MINUTE = 60_000;

/** The panes: this Mac's agent, and the two on the other computer. */
const LEAD = "lead";
const IMPORT = "m1";
const EXPORT = "m2";
const TITLE: Record<string, string> = { [LEAD]: "Fix the checkout timeout", [IMPORT]: "Run the nightly import", [EXPORT]: "Check the invoice export" };
const SESSION: Record<string, string> = { [LEAD]: "practice-lead", [IMPORT]: "practice-import", [EXPORT]: "practice-export" };

export type DevicesModel = {
  /** The computer in front: `local`, or the Mac mini's id. */
  front: "local" | typeof MINI;
  here: DeskModel;
  there: DeskModel;
  /** Whether the Mac mini has been brought forward, and whether the learner opened a tab on it. */
  visited: boolean;
  started: boolean;
};

/** Whether the learner brought the other computer forward. */
export const visitedThere = (model: DevicesModel) => model.front === MINI;
/** Whether a new tab was opened on the other computer. */
export const startedThere = (model: DevicesModel) => model.started;
/** Whether the learner is back on this Mac after working on the other. */
export const backHere = (model: DevicesModel) => model.started && model.front === "local";

const placeholder = (id: string, kind: string): PracticePane => ({ id, title: TITLE[id]!, agent: agent({ pane_id: id, identity_label: TITLE[id]!, agent_kind: kind, group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "3m", session_id: SESSION[id] }) });

function initial(): DevicesModel {
  const here = oneTabDesk(placeholder(LEAD, "claude"));
  const first = placeholder(IMPORT, "claude");
  const second = placeholder(EXPORT, "codex");
  const there: DeskModel = {
    panes: [first, second],
    tabs: [
      { id: "mt1", name: null, root: { type: "pane", pane_id: IMPORT }, focused: IMPORT, zoomed: false },
      { id: "mt2", name: null, root: { type: "pane", pane_id: EXPORT }, focused: EXPORT, zoomed: false },
    ],
    groups: [{ id: null, name: null, tabs: ["mt1", "mt2"] }],
    selectedGroup: null, activeTab: "mt1", nextId: 20, operations: [],
  };
  return { front: "local", here, there, visited: false, started: false };
}

// --- The events ----------------------------------------------------------------------------

/** The core's answer to a request sent to the Mac mini: a new tab holding a shell, a focus, a split. */
function remote(model: DevicesModel, p: Record<string, unknown>): DevicesModel | null {
  if (p.target_id !== MINI) return null;
  const desk = (event: CoreEvent) => applyDesk(model.there, event);
  const next = (there: DeskModel | null, started = model.started): DevicesModel | null => (there ? { ...model, there, started } : null);
  switch (p.action) {
    case "create_tab": return next(desk({ kind: "agent_group", payload: { group_id: null, action: "new_tab" } }), true);
    case "focus_tab": return next(desk({ kind: "focus_tab", payload: { tab_id: p.tab_id } }));
    case "focus_pane": return next(desk({ kind: "focus_pane", payload: { pane_id: p.pane_id } }));
    case "split_pane": {
      const tab = model.there.tabs.find((row) => panesOf(row.root).includes(String(p.pane_id)));
      return tab ? next(desk({ kind: "create_pane", payload: { tab_id: tab.id, direction: p.direction } })) : null;
    }
    default: return null;
  }
}

function apply(model: DevicesModel, event: CoreEvent): DevicesModel | null {
  const p = event.payload;
  if (event.kind === "focus_device") {
    const front = p.device_id === MINI ? MINI : p.device_id === "local" ? "local" : null;
    return front && front !== model.front ? { ...model, front, visited: model.visited || front === MINI } : null;
  }
  if (event.kind === "remote_control") return remote(model, p);
  // What the keyboard and the lists send follows the computer in front.
  const key = model.front === MINI ? "there" : "here";
  const desk = applyDesk(model[key], event);
  if (!desk) return null;
  return { ...model, [key]: desk, started: model.started || (key === "there" && desk.tabs.length > model.there.tabs.length) };
}

// --- The rows ------------------------------------------------------------------------------

const request = (id: string, at: number) => rowRequest(TITLE[id]!, TITLE[id]!, at, null);

function rowOf(id: string, paneId: string, nowMs: number, device: string): AgentRow {
  const base = { pane_id: paneId, identity_label: TITLE[id]!, session_id: SESSION[id], device_id: device, request: request(id, nowMs - 4 * MINUTE) };
  if (id === EXPORT) {
    const what = "Send the export to the finance team?";
    return agent({ ...base, agent_kind: "codex", group: "needs_you", mark: "turn", status_code: "question", demand: "question", activity: "idle", unread: true, elapsed: "40s", state: rowState("asking", true, nowMs - MINUTE, { ask: { verb: "answer", what, more: 0 }, line: { text: what, mode: "request", tone: { kind: "warning", read: false } } }) });
  }
  return agent({ ...base, agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "4m", requires_close_confirmation: true, state: rowState("working", true, nowMs - 4 * MINUTE) });
}

const headerOf = (id: string): PaneHeader => id === EXPORT
  ? { working: false, notice: null, band: { kind: "answer", reason: "Send the export to the finance team?", since_unix_ms: Date.now() - MINUTE, read: false, action: null, more: 0 }, request: { text: TITLE[id]!, images: 0 } }
  : { working: true, notice: null, band: null, request: { text: TITLE[id]!, images: 0 }, progress: id === LEAD ? "Editing src/payment/timeout.ts" : "Importing 4,210 invoices" };

/** The tasks a computer lists: its operator's turn first, then what works. */
function tasks(desk: DeskModel, place: PracticePlace, nowMs: number): ProjectTasksScope {
  const at = (minutes: number) => new Date(nowMs - minutes * MINUTE).toISOString();
  const checkout = `${place.project}:${place.branch}`;
  const cards = desk.panes.flatMap((pane) => {
    if (!pane.agent) return [];
    const asking = pane.id === EXPORT;
    return [{ root: pane.id, state: asking ? "needs_you" as const : "working" as const, members: [pane.id], checkouts: [checkout], recency: at(asking ? 1 : 4), stage: 1 as const }];
  }).sort((a, b) => Number(b.state === "needs_you") - Number(a.state === "needs_you"));
  return { cards, needs_you: cards.filter((card) => card.state === "needs_you").length, recency: at(0) };
}

/** A computer's scope with its tasks, and the counts its rail tile marks. */
function scopeOf(base: AgentScope, list: ProjectTasksScope, device: string | null = null): AgentScope {
  const working = list.cards.length - list.needs_you;
  // An agent of the other computer is listed from that computer's session, by its place there.
  const listed = device ? base.listed.map((entry, index) => ({ ...entry, device_id: device, device_label: "Mac mini", remote: true, index })) : base.listed;
  return { ...base, listed, tasks: list, groups: { needs_you: list.needs_you, done: 0, working, seen: 0 }, overview_needs_you: list.needs_you };
}

// --- The remote session --------------------------------------------------------------------

/** The rectangles a layout tree draws in, as the device's engine reports them. */
function framesOf(node: LayoutNode, x = 0, y = 0, width = 1, height = 1): RemotePaneFrame[] {
  if (node.type === "pane") return [{ pane_id: node.pane_id, x, y, width, height }];
  const across = node.direction === "right";
  const first = across ? node.ratio * width : width;
  const second = across ? width - first : width;
  const high = across ? height : node.ratio * height;
  const low = across ? height : height - high;
  return [...framesOf(node.first, x, y, first, high), ...framesOf(node.second, across ? x + first : x, across ? y : y + high, second, low)];
}

/** A project of the other computer as the core projects it: its device, and the device on each pane. */
function onMini(workspace: Workspace): Workspace {
  const checkouts = workspace.checkouts.map((checkout: Checkout) => ({ ...checkout, tabs: checkout.tabs.map((tab) => ({ ...tab, panes: tab.panes.map((pane) => ({ ...pane, device_id: MINI })) })) }));
  return { ...workspace, device_id: MINI, remote_target_id: MINI, checkouts };
}

function snapshot(model: DevicesModel, nowMs: number): WorldSnapshot {
  const hereRows = model.here.panes.map((pane) => ({ ...pane, agent: pane.agent && rowOf(pane.id, pane.id, nowMs, "local") }));
  const thereRows = model.there.panes.map((pane) => ({ ...pane, agent: pane.agent && rowOf(pane.id, pane.id, nowMs, MINI) }));
  const here = deskSnapshot([{ place: HERE, model: { ...model.here, panes: hereRows } }], HERE.project, nowMs);
  const there = deskSnapshot([{ place: THERE, model: { ...model.there, panes: thereRows } }], THERE.project, nowMs);
  const hereScope = scopeOf(here.rest.navigator!.devices![0]!.agent_scope, tasks(model.here, HERE, nowMs));
  const thereScope = scopeOf(there.rest.navigator!.devices![0]!.agent_scope, tasks(model.there, THERE, nowMs), MINI);
  const projects = there.rest.navigator!.workspaces!.map(onMini);
  const checkout = projects[0]!.checkouts[0]!;
  const agentsThere = there.agents.map((row) => ({ ...row, device_id: MINI }));
  const tab = model.there.tabs.find((row) => row.id === model.there.activeTab)!;
  const layouts: RemotePaneLayout[] = model.there.tabs.map((one) => ({ workspace_id: THERE.project, tab_id: one.id, focused_pane_id: one.focused, zoomed: one.zoomed, frames: framesOf(one.root) }));
  const local: SnapshotRest = here.rest;
  const front = model.front === MINI ? there.rest : here.rest;
  const headers: Record<string, PaneHeader> = Object.fromEntries([...model.here.panes, ...model.there.panes].map((pane) => [pane.id, headerOf(pane.id)]));
  const rest: SnapshotRest = {
    ...local,
    navigator: {
      ...local.navigator!,
      agent_scope: model.front === MINI ? thereScope : hereScope,
      devices: [
        { ...local.navigator!.devices![0]!, agent_scope: hereScope },
        { agent_scope: thereScope, id: MINI, label: "Mac mini", kind: "remote", state: "ready", message: null, ssh_alias: MINI, agent_count: agentsThere.length, test: null },
      ],
      focused_device_id: model.front,
    },
    ui_state: { ...local.ui_state, sidebar_lens: "tasks", device_rail_visible: true },
    status: {
      ...local.status,
      remote: [{
        target_id: MINI, state: "connected", message: null,
        session: {
          workspaces: projects, agents: agentsThere, active_tab_ids: { [checkout.id]: tab.id },
          focused_workspace_id: THERE.project, focused_checkout_id: checkout.id, focused_tab_id: tab.id, focused_pane_id: tab.focused, pane_layouts: layouts,
        },
      }],
    },
    pane_layouts: local.pane_layouts,
    workspace_view: model.front === MINI ? { ...front.workspace_view!, device_id: MINI } : local.workspace_view,
    terminal: { headers },
  };
  return { rest, agents: here.agents, focused: model.front === MINI ? focusedPane(model.there) : focusedPane(model.here), usage: null };
}

// --- What the terminals show ---------------------------------------------------------------

const CLAUDE = "\x1b[38;2;215;119;87m";
const CODEX = "\x1b[36m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const RESET = "\x1b[0m";

function screen(model: DevicesModel, paneId: string): string {
  if (paneId === LEAD) return [`${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${HERE.project} · ${HERE.branch}${RESET}`, "", `${DIM}>${RESET} Checkout times out on slow cards. Fix it`, "", `${CLAUDE}●${RESET} The timeout is fixed at three seconds.`, `  ${DIM}⎿${RESET}  Editing src/payment/timeout.ts…`].join("\r\n");
  if (paneId === IMPORT) return [`${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${THERE.project} · ${THERE.branch}${RESET}`, "", `${DIM}>${RESET} Run the nightly import`, "", `${CLAUDE}●${RESET} ${BOLD}Bash${RESET}(pnpm import:nightly)`, `  ${DIM}⎿${RESET}  Importing 4,210 invoices…`].join("\r\n");
  if (paneId === EXPORT) return [`${BOLD}>_ Codex${RESET}`, "", `${CODEX}›${RESET} ${TITLE[EXPORT]}`, "", `${DIM}•${RESET} ${BOLD}Ran${RESET} pnpm export:invoices`, `  ${DIM}└${RESET} 312 rows`, "", `${DIM}•${RESET} ${BOLD}Send the export to the finance team?${RESET}`, `  ${CODEX}› 1. Yes${RESET}   2. No`].join("\r\n");
  return shellScreen(model.there.panes.some((pane) => pane.id === paneId) ? THERE : HERE);
}

export const DEVICES_WORLD: PracticeWorld<DevicesModel> = {
  initial,
  apply,
  snapshot,
  // Only the computer in front draws terminals; the other's panes are not on screen.
  panes: (model) => (model.front === MINI ? model.there : model.here).panes.map((pane) => pane.id),
  screen: (model, paneId) => screen(model, paneId),
};
