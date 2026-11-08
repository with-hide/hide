// The several-agents flow's world: one invented project with a lead agent at
// work, a docs agent in a tab of its own, and what happens when the learner
// runs more agents than one: a second terminal beside, a reviewer the lead is
// asked for in words (an agent of its own, in a tab of its own), the docs
// agent finishing into Done while the reviewer asks into Needs You, the
// reviewer finishing and opening a file for the learner to look at (a View, the
// row's 볼 것), closing and reopening an agent, the keyboard going back to the
// agent used before, and the files the lead changed in the Explorer.
// Whatever words the learner sends the lead start the reviewer, since a
// practice understands no words; the reviewer's own turns end by themselves
// (`later`).

import type { ProjectTasksScope } from "../../agentScope";
import type { AgentRow, ChangesSnapshot, EditorDocumentSnapshot, EditorTabSnapshot, LayoutNode, PaneHeader, RecentClosed, ViewDisplaySnapshot } from "../../snapshot";
import type { DirectoryList } from "../../store";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, deskSnapshot, focusedPane, keyText, panesOf, rowRequest, rowState, shellScreen, typed, type DeskModel, type PracticePane, type PracticePlace, type PracticeTab } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };
const ROOT = `/work/${PLACE.project}`;
const CHECKOUT = `${PLACE.project}:${PLACE.branch}`;
const MINUTE = 60_000;

/** The panes as they first stand; a reopened agent comes back in a pane of another id, so the code asks for a pane's role. */
export const LEAD = "lead";
export const DOCS = "docs";
export const REV = "rev";
const REVIEWER_TAB = "t3";

type Role = "lead" | "docs" | "rev";
const SESSION: Record<Role, string> = { lead: "practice-lead", docs: "practice-docs", rev: "practice-rev" };
const TITLE: Record<Role, string> = { lead: "Fix the checkout timeout", docs: "Update the API docs", rev: "Review the checkout fix" };

const roleOf = (pane: PracticePane): Role | null => (["lead", "docs", "rev"] as const).find((role) => SESSION[role] === pane.agent?.session_id) ?? null;
const paneOf = (model: AgentsModel, role: Role) => model.panes.find((pane) => roleOf(pane) === role) ?? null;

/**
 * Where the script stands: 0 before the learner's words to the lead, 1 the
 * reviewer started, 2 the docs agent finished, 3 the reviewer asking, 4 the
 * learner's answer given and the reviewer at work, 5 the reviewer finished with
 * a View to look at, 6 the View looked at.
 */
export type AgentsStage = 0 | 1 | 2 | 3 | 4 | 5 | 6;

/** How long each of the agents' own turns takes before the next stage, by the stage it ends. */
export const BEATS: Partial<Record<AgentsStage, number>> = { 1: 2200, 2: 2200, 4: 2800 };
const BEAT = "practice_beat";

/** A closed pane as the core keeps it to reopen: the pane and its agent, and the tab it stood in as it was. */
type ClosedPane = { pane: PracticePane; tab: PracticeTab; group: string | null; position: number; order: number };

export type AgentsModel = DeskModel & {
  stage: AgentsStage;
  /** When each stage began, which paces the next beat. */
  times: Partial<Record<AgentsStage, number>>;
  /** What the learner has typed into the prompt of `draftIn` and not sent yet. */
  draft: string;
  draftIn: string | null;
  /** What the learner said to the lead, and the answer they gave the reviewer. */
  said: string[];
  answer: string | null;
  /** The agent panes, the one the keyboard was in last first, as the core keeps `recent_pane_ids`. */
  recent: string[];
  closed: ClosedPane[];
  /** The pane the last reopen brought back with what it was. */
  reopened: { paneId: string; tabId: string; from: ClosedPane } | null;
  /** Whether the learner opened the reviewer while it asked. */
  looked: boolean;
  /** Whether the keyboard went back to the lead after the reviewer came back. */
  switched: boolean;
  /** Whether the reviewer's file is open in the lane's View, and the core's count of lane calls. */
  viewed: boolean;
  calls: number;
  /** The Explorer shown beside the lane, and its open folders. */
  tool: "explorer" | null;
  expanded: string[];
};

/** Whether a second pane stands beside the lead in its tab. */
export const besideLead = (model: AgentsModel) => {
  const lead = paneOf(model, "lead");
  const tab = lead && model.tabs.find((row) => panesOf(row.root).includes(lead.id));
  return tab ? panesOf(tab.root).length >= 2 : false;
};
/** Whether the keyboard is in the pane of the reviewer. */
export const inReviewer = (model: AgentsModel) => paneOf(model, "rev")?.id === focusedPane(model);
export const showsExplorer = (model: AgentsModel) => model.tool === "explorer";
/** Whether the last reopen brought the closed pane back as it was: in its tab, with its agent and its title. */
export function reopenedAsClosed(model: AgentsModel): boolean {
  const back = model.reopened;
  const pane = back && model.panes.find((row) => row.id === back.paneId);
  const tab = back && model.tabs.find((row) => row.id === back.tabId);
  if (!back || !pane || !tab || !panesOf(tab.root).includes(pane.id)) return false;
  return pane.title === back.from.pane.title && pane.agent?.identity_label === back.from.pane.agent?.identity_label;
}

const since = (model: AgentsModel, stage: AgentsStage) => model.times[stage] ?? Date.now();

// --- The desk ------------------------------------------------------------------------------

/** A pane's agent as the desk keeps it before the snapshot draws it: its role (the session) and whether closing it asks. */
function placeholder(role: Role, paneId: string, kind: string, confirm: boolean): AgentRow {
  return agent({ pane_id: paneId, identity_label: TITLE[role], agent_kind: kind, group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "3m", session_id: SESSION[role], requires_close_confirmation: confirm });
}

function initial(): AgentsModel {
  const pane = (id: string, role: Role, kind: string, confirm: boolean): PracticePane => ({ id, title: TITLE[role], agent: placeholder(role, id, kind, confirm) });
  const tab = (id: string, paneId: string): PracticeTab => ({ id, name: null, root: { type: "pane", pane_id: paneId }, focused: paneId, zoomed: false });
  return {
    panes: [pane(LEAD, "lead", "claude", true), pane(DOCS, "docs", "codex", true)],
    tabs: [tab("t1", LEAD), tab("t2", DOCS)],
    groups: [{ id: null, name: null, tabs: ["t1", "t2"] }],
    selectedGroup: null, activeTab: "t1", nextId: 4, operations: [],
    stage: 0, times: { 0: Date.now() }, draft: "", draftIn: null, said: [], answer: null,
    recent: [LEAD, DOCS], closed: [], reopened: null, looked: false, switched: false, viewed: false, calls: 0, tool: null, expanded: [],
  };
}

/** The reviewer the lead starts: an agent of its own in a tab of its own, which Hide leaves behind the screen the learner is on. */
function startReviewer(model: AgentsModel): AgentsModel {
  const rev: PracticePane = { id: REV, title: TITLE.rev, agent: placeholder("rev", REV, "codex", false) };
  return {
    ...model,
    panes: [...model.panes, rev],
    tabs: [...model.tabs, { id: REVIEWER_TAB, name: null, root: { type: "pane", pane_id: REV }, focused: REV, zoomed: false }],
    groups: model.groups.map((group) => (group.id === null ? { ...group, tabs: [...group.tabs, REVIEWER_TAB] } : group)),
    recent: [...model.recent, REV],
  };
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
 * The model after the pane `id` closed. A working agent closes only once the
 * operator confirmed it on the Stop-work sheet; its tab's last pane closes the
 * tab, and the area then shows the tab before it in its group. The desk's last
 * pane is not closed here.
 */
function closePane(model: AgentsModel, id: string, confirmed: boolean): AgentsModel | null {
  const pane = model.panes.find((row) => row.id === id);
  const tab = model.tabs.find((row) => panesOf(row.root).includes(id));
  if (!pane || !tab || model.panes.length === 1) return null;
  if (pane.agent?.requires_close_confirmation && !confirmed) return null;
  const group = model.groups.find((row) => row.tabs.includes(tab.id))!;
  const closed: ClosedPane = { pane, tab, group: group.id, position: group.tabs.indexOf(tab.id), order: model.tabs.indexOf(tab) };
  const panes = model.panes.filter((row) => row !== pane);
  const recent = model.recent.filter((one) => one !== id);
  const root = without(tab.root, id);
  if (root) {
    const focused = tab.focused === id ? panesOf(root)[0]! : tab.focused;
    return { ...model, panes, recent, tabs: model.tabs.map((row) => (row === tab ? { ...row, root, focused, zoomed: false } : row)), closed: [...model.closed, closed] };
  }
  const tabs = model.tabs.filter((row) => row !== tab);
  const groups = model.groups.map((row) => ({ ...row, tabs: row.tabs.filter((one) => one !== tab.id) }));
  const siblings = group.tabs.filter((one) => one !== tab.id);
  const front = model.activeTab !== tab.id ? model.activeTab : (siblings[closed.position - 1] ?? siblings[closed.position] ?? tabs[0]!.id);
  return { ...model, panes, recent, tabs, groups, activeTab: front, selectedGroup: groups.find((row) => row.tabs.includes(front))!.id, closed: [...model.closed, closed] };
}

/**
 * The newest close brought back in a new pane, as the engine makes one, holding
 * the same agent continuing its conversation: in its tab where it stood when
 * the tab is as the close left it, beside the tab's focused pane when the tab
 * changed since, and as a tab of its own in its former group and place when the
 * tab closed with it.
 */
function reopen(model: AgentsModel): AgentsModel | null {
  const last = model.closed.at(-1);
  if (!last) return null;
  const paneId = `${last.pane.agent ? "ag" : "sh"}${model.nextId}`;
  const pane: PracticePane = { ...last.pane, id: paneId, agent: last.pane.agent && { ...last.pane.agent, pane_id: paneId, id: paneId } };
  const recent = pane.agent ? [paneId, ...model.recent] : model.recent;
  const base = { ...model, panes: [...model.panes, pane], recent, nextId: model.nextId + 1, closed: model.closed.slice(0, -1) };
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

// --- The script ----------------------------------------------------------------------------

/** The model moved to `stage`, which begins now. */
const to = (model: AgentsModel, stage: AgentsStage): AgentsModel => ({ ...model, stage, times: { ...model.times, [stage]: Date.now() } });

/** The prompt of `paneId`, sent: the lead's first words start the reviewer, the reviewer's answer to its question lets it go on. */
function submit(model: AgentsModel, paneId: string): AgentsModel {
  const text = model.draft.trim();
  const cleared = { ...model, draft: "", draftIn: null };
  if (!text) return cleared;
  const role = roleOf(model.panes.find((pane) => pane.id === paneId)!);
  if (role === "lead" && model.stage === 0) return to({ ...startReviewer(cleared), said: [...model.said, text] }, 1);
  if (role === "rev" && model.stage === 3) return to({ ...cleared, answer: text }, 4);
  return role === "lead" ? { ...cleared, said: [...model.said, text] } : cleared;
}

/** The pane moved to the front of the recent order when it runs an agent. */
function visited(model: AgentsModel): AgentsModel {
  const pane = focusedPane(model);
  const recent = model.recent.includes(pane) && model.recent[0] !== pane ? [pane, ...model.recent.filter((id) => id !== pane)] : model.recent;
  // Coming back to the lead after the reviewer has been brought back again is the keyboard going to the agent used before.
  const switched = model.switched || (model.reopened !== null && paneOf(model, "lead")?.id === pane);
  return recent === model.recent && switched === model.switched ? model : { ...model, recent, switched };
}

function apply(model: AgentsModel, event: CoreEvent): AgentsModel | null {
  const p = event.payload;
  switch (event.kind) {
    case BEAT:
      return p.stage === model.stage + 1 && BEATS[model.stage] !== undefined ? to(model, p.stage as AgentsStage) : null;
    // Only the lead and, while it asks, the reviewer are talked to here; keys typed into another pane reach no one.
    case "key": {
      const id = String(p.pane_id);
      const pane = model.panes.find((row) => row.id === id);
      const role = pane && roleOf(pane);
      if (typeof p.bytes_base64 !== "string" || (role !== "lead" && role !== "rev")) return null;
      const base = model.draftIn === id ? model : { ...model, draft: "", draftIn: id };
      return typed(base, keyText(p.bytes_base64), (one) => submit(one, id));
    }
    case "focus_pane": {
      const id = String(p.pane_id);
      const desk = applyDesk(model, event);
      const pane = model.panes.find((row) => row.id === id);
      if (!desk || !pane) return null;
      let next: AgentsModel = { ...model, ...desk };
      const role = roleOf(pane);
      if (role === "rev" && model.stage === 3) next = { ...next, looked: true };
      // The row with a View to look at brings the View forward with the pane, which sees it.
      if (role === "rev" && model.stage === 5 && p.reveal_unseen === true) next = to({ ...next, viewed: true, calls: model.calls + 1 }, 6);
      return visited(next);
    }
    case "focus_tab": {
      const desk = applyDesk(model, event);
      return desk ? visited({ ...model, ...desk }) : null;
    }
    case "close_pane":
      return closePane(model, String(p.pane_id), p.confirmed === true);
    // ⌘W closes the tab holding the keyboard; a practice tab holds one pane, so that pane closes with it.
    case "close_tab": {
      const panes = panesOf(model.tabs.find((tab) => tab.id === p.tab_id)?.root ?? { type: "pane", pane_id: "" });
      return panes.length === 1 ? closePane(model, panes[0]!, p.confirmed === true) : null;
    }
    case "reopen_closed": {
      const back = reopen(model);
      return back && visited(back);
    }
    // The Explorer shown or taken down, or the lane put away with what it showed.
    case "workspace_view": {
      let tool = model.tool;
      if ("tool" in p) tool = p.tool === "explorer" ? "explorer" : null;
      if (typeof p.reveal === "string") tool = "explorer";
      if (p.open === false) tool = null;
      const viewed = p.open === false ? false : model.viewed;
      return tool === model.tool && viewed === model.viewed ? null : { ...model, tool, viewed };
    }
    // The Explorer folds a folder through the core's ui state.
    case "ui_state_update": {
      if (!Array.isArray(p.expanded_paths)) return null;
      const expanded = p.expanded_paths.filter((path): path is string => typeof path === "string");
      return expanded.join("\n") === model.expanded.join("\n") ? null : { ...model, expanded };
    }
    default: {
      const desk = applyDesk(model, event);
      return desk && visited({ ...model, ...desk });
    }
  }
}

// --- The rows ------------------------------------------------------------------------------

const request = (role: Role, at: number, extra = {}) => rowRequest(TITLE[role], TITLE[role], at, null, extra);

/** The agent row of `role` in `paneId` at the model's stage, as the core would send it. */
function rowOf(model: AgentsModel, role: Role, paneId: string, nowMs: number): AgentRow {
  const stage = model.stage;
  const base = { pane_id: paneId, identity_label: TITLE[role], session_id: SESSION[role] };
  if (role === "lead") {
    return agent({ ...base, agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "3m", requires_close_confirmation: true, state: rowState("working", true, nowMs - 3 * MINUTE), request: request("lead", nowMs - 3 * MINUTE) });
  }
  if (role === "docs") {
    const at = nowMs - 8 * MINUTE;
    return stage >= 2
      ? agent({ ...base, agent_kind: "codex", group: "done", mark: "done", status_code: "done", activity: "stopped", elapsed: "1m", state: rowState("done", true, at, { session: { group: "done", line: "The API docs are current", unfinished: false } }), request: request("docs", at, { end: "done", line: "The API docs are current" }) })
      : agent({ ...base, agent_kind: "codex", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "8m", requires_close_confirmation: true, state: rowState("working", true, at), request: request("docs", at) });
  }
  const at = since(model, 1);
  const common = { ...base, agent_kind: "codex", request: request("rev", at) };
  if (stage === 3) {
    const what = "Run the full test suite?";
    return agent({ ...common, group: "needs_you", mark: "turn", status_code: "question", demand: "question", activity: "idle", unread: true, elapsed: "2s", state: rowState("asking", true, since(model, 3), { ask: { verb: "answer", what, more: 0 }, line: { text: what, mode: "request", tone: { kind: "warning", read: false } } }) });
  }
  if (stage === 5) {
    return agent({
      ...common, group: "needs_you", mark: "look", status_code: "done", activity: "stopped", unread: true, elapsed: "3s",
      state: rowState("look", true, since(model, 5)),
      look: { group: "done", count: 1, view_id: REVIEW_VIEW, label: REVIEW_FILE, workspace_id: PLACE.project, checkout_id: CHECKOUT },
    });
  }
  if (stage === 6) return agent({ ...common, group: "seen", mark: "quiet", status_code: "done", activity: "stopped", elapsed: "10s", state: rowState("quiet", true, since(model, 6)) });
  return agent({ ...common, group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "1m", state: rowState("working", true, at) });
}

const REVIEW_VIEW = "view-review";
const REVIEW_FILE = "review-notes.md";
const REVIEW_PATH = `${ROOT}/${REVIEW_FILE}`;

/** What each pane's head says on its second row. */
function headerOf(model: AgentsModel, role: Role | null): PaneHeader {
  if (role === "lead") return { working: true, notice: null, band: null, request: { text: TITLE.lead, images: 0 }, progress: "Editing src/payment/timeout.ts" };
  if (role === "docs") {
    return model.stage >= 2
      ? { working: false, notice: null, band: null, request: { text: TITLE.docs, images: 0 }, finished: true, progress: "The API docs are current" }
      : { working: true, notice: null, band: null, request: { text: TITLE.docs, images: 0 }, progress: "Writing docs/api.md" };
  }
  if (role === "rev") {
    if (model.stage === 3) return { working: false, notice: null, band: { kind: "answer", reason: "Run the full test suite?", since_unix_ms: since(model, 3), read: model.looked, action: null, more: 0 }, request: { text: TITLE.rev, images: 0 } };
    if (model.stage >= 5) return { working: false, notice: null, band: null, request: { text: TITLE.rev, images: 0 }, finished: true, progress: "The review notes are ready" };
    return { working: true, notice: null, band: null, request: { text: TITLE.rev, images: 0 }, progress: "Reading src/payment/timeout.ts" };
  }
  return { working: false, notice: null, band: null };
}

/** The tasks the list shows, in the core's order: the operator's turn first. */
function tasks(model: AgentsModel, nowMs: number): ProjectTasksScope {
  const at = (minutes: number) => new Date(nowMs - minutes * MINUTE).toISOString();
  const card = (role: Role, state: "needs_you" | "working" | "done" | "resting", minutes: number, extra: { read?: true } = {}) => {
    const pane = paneOf(model, role);
    return pane ? [{ root: pane.id, state, members: [pane.id], checkouts: [CHECKOUT], recency: at(minutes), stage: 1 as const, ...extra }] : [];
  };
  const stage = model.stage;
  const rev = stage === 3 ? card("rev", "needs_you", 0, model.looked ? { read: true } : {}) : stage === 5 ? card("rev", "needs_you", 0) : stage === 6 ? card("rev", "resting", 0) : card("rev", "working", 1);
  const docs = card("docs", stage >= 2 ? "done" : "working", stage >= 2 ? 1 : 8);
  const cards = [...rev.filter((one) => one.state === "needs_you"), ...card("lead", "working", 3), ...rev.filter((one) => one.state !== "needs_you"), ...docs];
  return { cards, needs_you: cards.filter((one) => one.state === "needs_you").length, recency: at(0) };
}

// --- The checkout's files ------------------------------------------------------------------

/** Every file of the checkout, relative to it. */
const FILES = ["README.md", "package.json", "docs/api.md", "src/payment/checkout.ts", "src/payment/retry.ts", "src/payment/timeout.ts", "test/checkout.test.ts", REVIEW_FILE];

/** Each folder's listing as the core answers it, folders first. */
function listings(): Record<string, DirectoryList> {
  const folders = new Set([""]);
  for (const file of FILES) file.split("/").slice(0, -1).forEach((_, at, parts) => folders.add(parts.slice(0, at + 1).join("/")));
  return Object.fromEntries([...folders].map((folder) => {
    const path = folder ? `${ROOT}/${folder}` : ROOT;
    const depth = folder ? folder.split("/").length + 1 : 1;
    const inside = (name: string) => (folder ? name.startsWith(`${folder}/`) : true);
    const subfolders = [...folders].filter((one) => one && inside(one) && one.split("/").length === depth);
    const files = FILES.filter((file) => inside(file) && file.split("/").length === depth);
    const entries = [...subfolders.sort().map((one) => ({ name: one.split("/").at(-1)!, path: `${ROOT}/${one}`, is_directory: true })), ...files.sort().map((one) => ({ name: one.split("/").at(-1)!, path: `${ROOT}/${one}`, is_directory: false }))];
    return [path, { kind: "file_list" as const, root_path: path, entries, truncated: false }];
  }));
}

/** What the lead has changed so far, uncommitted. */
const CHANGED = [
  { file: "src/payment/checkout.ts", added: 12, removed: 3, fresh: false },
  { file: "src/payment/timeout.ts", added: 9, removed: 0, fresh: true },
  { file: "test/checkout.test.ts", added: 18, removed: 0, fresh: false },
];

function changes(): ChangesSnapshot {
  return {
    root_path: ROOT,
    entries: CHANGED.map((one) => ({ path: `${ROOT}/${one.file}`, relative_path: one.file, previous_relative_path: null, status: one.fresh ? "untracked" as const : "modified" as const, added_lines: one.added, removed_lines: one.removed })),
    committed: [], base_branch: "main", selected_path: null, selected_committed: false, diff: null, unavailable_reason: null,
  };
}

const EDITOR_TAB = "file1";

const NOTES = [
  "# Review of the checkout fix",
  "",
  "- The timeout now reads from settings, as asked.",
  "- A slow card still has no test. Add one before merging.",
  "",
].join("\n");

/** The lane as the core draws it: the Explorer beside the View, and the reviewer's notes as the View's one display. */
function viewOf(model: AgentsModel, view: NonNullable<WorldSnapshot["rest"]["workspace_view"]>): NonNullable<WorldSnapshot["rest"]["workspace_view"]> {
  const lane = { ...view, open: model.tool !== null || model.viewed, tool: model.tool, lane_called: model.calls, lane_calls: model.calls, lane_called_side: "view" as const };
  if (!model.viewed) return lane;
  const display: ViewDisplaySnapshot = { id: "view1", tab_id: EDITOR_TAB, path: REVIEW_PATH, label: REVIEW_FILE, kind: "file", committed: null, preview: true, state: "open", reason: null };
  return { ...lane, documents: true, layout: { root: { area: { id: "v1", active: display.id, displays: [display] } }, active_area: "v1", display_count: 1, limits: { areas: 4, depth: 3, displays: 32 } } };
}

function opened(model: AgentsModel): { editor?: { tabs: EditorTabSnapshot[]; active_tab_id: string }; documents?: Record<string, EditorDocumentSnapshot> } {
  if (!model.viewed) return {};
  const tab: EditorTabSnapshot = { id: EDITOR_TAB, device_id: "local", workspace_id: PLACE.project, checkout_id: CHECKOUT, path: REVIEW_PATH, label: REVIEW_FILE, kind: "file", diff_committed: null, markdown_live: true, wrap: false, dirty: false, preview: true };
  const document: EditorDocumentSnapshot = { path: REVIEW_PATH, language: "markdown", document_kind: "markdown", contents_utf8: NOTES, revision: "sha256:0", dirty: false, readonly_reason: null, conflict: null, save: null };
  return { editor: { tabs: [tab], active_tab_id: EDITOR_TAB }, documents: { [EDITOR_TAB]: document } };
}

// --- The snapshot --------------------------------------------------------------------------

/** The core's recent-close stack as the snapshot carries it, which the shell's Reopen closed tab reads before it sends. */
function recentClosed(model: AgentsModel): RecentClosed {
  const top = model.closed.at(-1);
  return { count: model.closed.length, top_label: top?.pane.title ?? null, restoring: false, pending: [], can_reopen: top !== undefined, reopen_blocked_reason: null };
}

function snapshot(model: AgentsModel, nowMs: number): WorldSnapshot {
  const rows: Record<string, AgentRow> = {};
  const headers: Record<string, PaneHeader> = {};
  for (const pane of model.panes) {
    const role = roleOf(pane);
    headers[pane.id] = headerOf(model, role);
    if (role) rows[pane.id] = rowOf(model, role, pane.id, nowMs);
  }
  const desk: DeskModel = { ...model, panes: model.panes.map((pane) => ({ ...pane, agent: rows[pane.id] ?? null })) };
  const drawn = deskSnapshot([{ place: PLACE, model: desk }], PLACE.project, nowMs);
  const navigator = drawn.rest.navigator!;
  const scope = { ...navigator.devices![0]!.agent_scope, tasks: tasks(model, nowMs) };
  return {
    ...drawn,
    rest: {
      ...drawn.rest,
      navigator: { ...navigator, devices: navigator.devices!.map((device) => ({ ...device, agent_scope: scope })), changes_root_path: ROOT },
      ui_state: { ...drawn.rest.ui_state, sidebar_lens: "tasks", recent_pane_ids: model.recent, expanded_paths: model.expanded },
      workspace_view: viewOf(model, drawn.rest.workspace_view!),
      recent_closed: recentClosed(model),
      terminal: { headers },
    },
    frames: { listings: listings(), changes: changes(), ...opened(model) },
  };
}

// --- What the terminals show ---------------------------------------------------------------

const CLAUDE = "\x1b[38;2;215;119;87m";
const CODEX = "\x1b[36m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const GREEN = "\x1b[32m";
const RESET = "\x1b[0m";
const rule = (cols: number) => `${DIM}${"─".repeat(cols)}${RESET}`;
const said = (text: string) => ["", `${DIM}>${RESET} ${text}`, ""];

function leadScreen(model: AgentsModel, cols: number): string {
  const rows = [
    "", `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`,
    ...said("Checkout times out on slow cards. Fix it"),
    `${CLAUDE}●${RESET} The timeout is fixed at three seconds.`,
    `  ${DIM}⎿${RESET}  Editing src/payment/timeout.ts…`,
  ];
  if (model.stage >= 1) {
    rows.push(...said(model.said[0]!), `${CLAUDE}●${RESET} ${BOLD}Bash${RESET}(hide agent spawn --name reviewer …)`, `  ${DIM}⎿${RESET}  Started ${TITLE.rev} in a tab of its own`);
    for (const later of model.said.slice(1)) rows.push(...said(later), `${CLAUDE}●${RESET} Noted.`);
  }
  rows.push("", `${CLAUDE}✻ ${DIM}Working… (esc to interrupt)${RESET}`, "", rule(cols), `${DIM}>${RESET} ${model.draftIn === paneOf(model, "lead")?.id ? model.draft : ""}`, rule(cols));
  return rows.join("\r\n");
}

function docsScreen(model: AgentsModel): string {
  return [
    `${BOLD}>_ Codex${RESET}`, "", `${CODEX}›${RESET} ${TITLE.docs}`, "",
    `${DIM}•${RESET} ${BOLD}Edited${RESET} docs/api.md`,
    ...(model.stage >= 2 ? [`  ${GREEN}+ The timeout setting${RESET}`, "", `${DIM}•${RESET} The API docs are current.`] : [`  ${DIM}└${RESET} writing…`]),
  ].join("\r\n");
}

function revScreen(model: AgentsModel, cols: number): string {
  const rows = [`${BOLD}>_ Codex${RESET}`, "", `${CODEX}›${RESET} ${TITLE.rev}`, "", `${DIM}•${RESET} ${BOLD}Read${RESET} src/payment/timeout.ts`, `  ${DIM}└${RESET} the timeout now comes from settings`];
  if (model.stage >= 3) rows.push("", `${DIM}•${RESET} ${BOLD}Run the full test suite?${RESET}`, `  ${CODEX}› 1. Yes${RESET}   2. No`);
  if (model.stage >= 4) rows.push(...said(model.answer ?? ""), `${DIM}•${RESET} ${BOLD}Ran${RESET} pnpm test`, `  ${DIM}└${RESET} ${GREEN}41 passed${RESET}`);
  if (model.stage >= 5) rows.push("", `${DIM}•${RESET} ${BOLD}Ran${RESET} hide file open ${REVIEW_FILE}`, `  ${DIM}└${RESET} opened for you to look at`, "", `${DIM}•${RESET} The review notes are ready.`);
  rows.push("", rule(cols), `${DIM}›${RESET} ${model.draftIn === paneOf(model, "rev")?.id ? model.draft : ""}`);
  return rows.join("\r\n");
}

export const AGENTS_WORLD: PracticeWorld<AgentsModel> = {
  initial,
  apply,
  snapshot,
  prompt: (model, paneId) => (model.draftIn === paneId ? model.draft : ""),
  // The agents' own turns end by themselves, as agents at work would.
  later: (model) => {
    const wait = BEATS[model.stage];
    return wait === undefined ? null : { at: since(model, model.stage) + wait, event: { kind: BEAT, payload: { stage: model.stage + 1 } } };
  },
  panes: (model) => model.panes.map((pane) => pane.id),
  screen: (model, paneId, cols) => {
    const pane = model.panes.find((row) => row.id === paneId)!;
    const role = roleOf(pane);
    return role === "lead" ? leadScreen(model, cols) : role === "docs" ? docsScreen(model) : role === "rev" ? revScreen(model, cols) : shellScreen(PLACE);
  },
};
