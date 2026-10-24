// Chapter 3's world: a lead agent fixing a checkout bug with two Codex
// children, each in its own tab as delegated children are, beside a second
// project whose tasks ask and finish. Opening a child and coming back to the
// lead is the one script it plays: back at the lead, the cause child has
// finished and the lead asks the operator, so the list and the head show work
// going from working to the operator's turn to done.

import type { ProjectTasksScope } from "../../agentScope";
import type { AgentChip, AgentRequest, AgentRow, AgentState, PaneFocusRequest, PaneHeader, StatusTone } from "../../snapshot";
import type { PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, deskSnapshot, shellScreen, type DeskModel, type PracticePane, type PracticePlace } from "./kit";

const SHOP: PracticePlace = { project: "shop-app", label: "shop-app", branch: "main" };
const SITE: PracticePlace = { project: "website", label: "website", branch: "main" };

/** Where the script stands: before the learner came back to the lead, or after. */
type Beat = "working" | "answered";

export type FollowModel = {
  shop: DeskModel;
  site: DeskModel;
  front: string;
  beat: Beat;
  /** The core's receipt for the last focus asked by relationship. */
  focus: PaneFocusRequest | null;
};

const LEAD = "lead";
const CAUSE = "cause";
const REPRO = "repro";
const SIGNUP = "signup";
const README = "readme";

const TITLES: Record<string, string> = {
  [LEAD]: "Fix the checkout 500",
  [CAUSE]: "Find the cause",
  [REPRO]: "Write a repro test",
  [SIGNUP]: "Approve the signup copy?",
  [README]: "Update the README",
};

const MINUTE = 60_000;

const tone = (kind: StatusTone["kind"], read = false): StatusTone => ({ kind, read });

/**
 * A row's state as the core states it (`row_state`), for the four ways a row
 * stands here: working, asking, done (a root's unread result) and quiet (a
 * finished child, which draws no mark of the operator's).
 */
function state(kind: "working" | "asking" | "done" | "quiet", root: boolean, since: number, extra: Partial<AgentState> = {}): AgentState {
  const base: AgentState = {
    attention: false, needs_you: false, root, title_emphasized: false, selection_emphasizes_title: true,
    asking: false, working: true, waits_on_children: false,
    chip_tone: tone("working"), mark_tone: tone("working"),
    line: null, branch_badge: null, bucket: "working", attention_rank: 3, link_rank: 1, search_tone: "working", subtree: "working", link: "working",
    descendant_asking: 0, verb: "working", request_todo: false, request_since: since,
    session: { group: "working", line: null, unfinished: false }, ask: null, tree_rank: 1, pr: null,
  };
  if (kind === "asking") {
    Object.assign(base, {
      attention: true, needs_you: true, title_emphasized: true, asking: true, working: false,
      chip_tone: tone("warning"), mark_tone: tone("warning"), bucket: "turn", attention_rank: 1, link_rank: 0, search_tone: "attention", subtree: "waiting", link: "question",
      verb: "answer", session: { group: "needs_you", line: null, unfinished: false }, tree_rank: 0,
    } satisfies Partial<AgentState>);
  }
  if (kind === "done") {
    Object.assign(base, {
      title_emphasized: root, working: false,
      chip_tone: tone("success"), mark_tone: tone("success"), bucket: "turn", attention_rank: 2, link_rank: 2, search_tone: "done", subtree: "unread", link: "idle",
      verb: "result", session: { group: "done", line: null, unfinished: false }, tree_rank: 2,
    } satisfies Partial<AgentState>);
  }
  if (kind === "quiet") {
    Object.assign(base, {
      working: false,
      chip_tone: tone("subtle"), mark_tone: tone("subtle"), bucket: "resting", attention_rank: 4, link_rank: 2, search_tone: "muted", subtree: "quiet", link: "idle",
      verb: "idle", session: { group: "idle", line: null, unfinished: false }, tree_rank: 3,
    } satisfies Partial<AgentState>);
  }
  return { ...base, ...extra };
}

/** The request a row shows: the operator's for a root, the lead's for a child. */
function request(text: string, summary: string, at: number, fromLead: boolean, extra: Partial<AgentRequest> = {}): AgentRequest {
  return {
    verb: "working", verb_since_unix_ms: at, later_by: null, pull_requests: [], request_summary: summary,
    request: { text, cut: false, images: 0, at_unix_ms: at, sender: fromLead ? { kind: "named", name: TITLES[LEAD]! } : { kind: "operator" } },
    ...extra,
  };
}

const counts = (working: number, done: number, question = 0) => ({ error: 0, approval: 0, question, working, done });

/** Every agent row for `beat`, as the core would send them at `nowMs`. */
function rows(beat: Beat, nowMs: number): Record<string, AgentRow> {
  const answered = beat === "answered";
  const asked = nowMs - 6 * MINUTE;
  const lead = agent({
    pane_id: LEAD, identity_label: TITLES[LEAD]!, agent_kind: "claude", session_id: "practice-lead",
    ...(answered
      ? { group: "needs_you", mark: "turn", status_code: "question", demand: "question", activity: "idle", unread: true, elapsed: "5s",
        state: state("asking", true, asked, { ask: { verb: "answer", what: "Fill the empty currency for saved cards?", more: 0 }, line: { text: "Fill the empty currency for saved cards?", mode: "request", tone: tone("warning") } }) }
      : { group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "2m",
        state: state("working", true, asked, { waits_on_children: true, line: { text: "", mode: "waiting", tone: tone("working", true) } }), wait: "children" as const }),
    lineage_child_pane_ids: [CAUSE, REPRO],
    direct_child_counts: answered ? counts(1, 1) : counts(2, 0),
    descendant_counts: answered ? counts(1, 1) : counts(2, 0),
    descendant_mark: { kind: "working", count: answered ? 1 : 2 },
    request: request("Checkout returns a 500 when I pay. Fix it", "Fix the 500 when paying at checkout", asked, false, {
      recent_turns: [{ request: "Look at last night's payment errors", end: "done", line: "They start with the 10:42 deploy", at_unix_ms: asked - 20 * MINUTE }],
    }),
  });
  const cause = agent({
    pane_id: CAUSE, identity_label: TITLES[CAUSE]!, agent_kind: "codex", session_id: "practice-cause", delegated: true,
    lineage_parent_pane_id: LEAD, lineage_path_pane_ids: [LEAD, CAUSE], lineage_depth: 1,
    ...(answered
      ? { group: "seen", mark: "quiet", status_code: "done", activity: "stopped", elapsed: "10s", state: state("quiet", false, asked + MINUTE) }
      : { group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "1m", state: state("working", false, asked + MINUTE, { root: false }) }),
    request: request("Find why checkout returns a 500", "Find why checkout returns a 500", asked + MINUTE, true, answered ? { end: "done", line: "Saved cards have an empty currency" } : {}),
  });
  const repro = agent({
    pane_id: REPRO, identity_label: TITLES[REPRO]!, agent_kind: "codex", session_id: "practice-repro", delegated: true,
    lineage_parent_pane_id: LEAD, lineage_path_pane_ids: [LEAD, REPRO], lineage_depth: 1,
    group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "40s",
    state: state("working", false, asked + MINUTE),
    request: request("Write a test that reproduces the 500", "Write a test that reproduces the 500", asked + MINUTE, true),
  });
  const signup = agent({
    pane_id: SIGNUP, identity_label: TITLES[SIGNUP]!, agent_kind: "codex", session_id: "practice-signup",
    group: "needs_you", mark: "turn", status_code: "question", demand: "question", activity: "idle", unread: true, elapsed: "3m",
    state: state("asking", true, nowMs - 9 * MINUTE, { ask: { verb: "answer", what: "Ship the shorter headline?", more: 0 }, line: { text: "Ship the shorter headline?", mode: "request", tone: tone("warning") } }),
    request: request("Rewrite the signup page copy", "Rewrite the signup page copy", nowMs - 9 * MINUTE, false),
  });
  const readme = agent({
    pane_id: README, identity_label: TITLES[README]!, agent_kind: "claude", session_id: "practice-readme",
    group: "done", mark: "done", status_code: "done", activity: "stopped", elapsed: "12m",
    state: state("done", true, nowMs - 30 * MINUTE, { session: { group: "done", line: "Install steps are current", unfinished: false } }),
    request: request("Bring the README up to date", "Bring the README up to date", nowMs - 30 * MINUTE, false, { end: "done", line: "Install steps are current" }),
  });
  return { [LEAD]: lead, [CAUSE]: cause, [REPRO]: repro, [SIGNUP]: signup, [README]: readme };
}

/** The lead's request as its head shows it: the summary of what the operator asked. */
const LEAD_REQUEST = { text: "Fix the 500 when paying at checkout", images: 0 };

/** What each pane's head says on its second row. */
function headers(beat: Beat, nowMs: number): Record<string, PaneHeader> {
  const answered = beat === "answered";
  return {
    [LEAD]: answered
      ? { working: false, notice: null, band: { kind: "answer", reason: "Fill the empty currency for saved cards?", since_unix_ms: nowMs - 5_000, read: false, action: null, more: 0 }, request: LEAD_REQUEST }
      : { working: true, notice: null, band: null, request: LEAD_REQUEST, waiting_on_children: 2 },
    [CAUSE]: answered
      ? { working: false, notice: null, band: null, request: { text: "Find why checkout returns a 500", images: 0 }, finished: true, progress: "Saved cards have an empty currency" }
      : { working: true, notice: null, band: null, request: { text: "Find why checkout returns a 500", images: 0 }, progress: "Reading the payment logs" },
    [REPRO]: { working: true, notice: null, band: null, request: { text: "Write a test that reproduces the 500", images: 0 }, progress: "Writing test/checkout.test.ts" },
    [SIGNUP]: { working: false, notice: null, band: { kind: "answer", reason: "Ship the shorter headline?", since_unix_ms: nowMs - 3 * MINUTE, read: false, action: null, more: 0 } },
    [README]: { working: false, notice: null, band: null, progress: "Install steps are current" },
  };
}

/** A child as its parent's lineage lists it. */
function chip(row: AgentRow): AgentChip {
  return {
    tone: row.state.chip_tone, pane_id: row.pane_id, label: row.identity_label, detail: null, status_word_visible: false,
    agent_kind: row.agent_kind, demand: row.demand ?? "none", activity: row.activity ?? "idle", emphasized: row.emphasized, mark: row.mark, status_code: row.status_code, delegated: true,
  };
}

/** The tasks the device lists, in the core's order: the operator's turn first. */
function tasks(beat: Beat, nowMs: number): ProjectTasksScope {
  const at = (minutes: number) => new Date(nowMs - minutes * MINUTE).toISOString();
  const lead = { root: LEAD, state: beat === "answered" ? "needs_you" as const : "working" as const, members: [LEAD, CAUSE, REPRO], checkouts: [`${SHOP.project}:${SHOP.branch}`], recency: at(beat === "answered" ? 0 : 2), stage: 1 as const };
  const signup = { root: SIGNUP, state: "needs_you" as const, members: [SIGNUP], checkouts: [`${SITE.project}:${SITE.branch}`], recency: at(3), stage: 1 as const };
  const readme = { root: README, state: "done" as const, members: [README], checkouts: [`${SITE.project}:${SITE.branch}`], recency: at(12), stage: 1 as const };
  const cards = beat === "answered" ? [lead, signup, readme] : [signup, lead, readme];
  return { cards, needs_you: cards.filter((card) => card.state === "needs_you").length, recency: at(0) };
}

const pane = (id: string): PracticePane => ({ id, agent: null, title: TITLES[id]! });
const tab = (id: string, paneId: string) => ({ id, name: null, root: { type: "pane" as const, pane_id: paneId }, focused: paneId, zoomed: false });

function initial(): FollowModel {
  const shop: DeskModel = {
    panes: [pane(LEAD), pane(CAUSE), pane(REPRO)],
    tabs: [tab("t1", LEAD), tab("t2", CAUSE), tab("t3", REPRO)],
    groups: [{ id: null, name: null, tabs: ["t1", "t2", "t3"] }],
    selectedGroup: null, activeTab: "t1", nextId: 4, operations: [],
  };
  const site: DeskModel = {
    panes: [pane(SIGNUP), pane(README)],
    tabs: [tab("s1", SIGNUP), tab("s2", README)],
    groups: [{ id: null, name: null, tabs: ["s1", "s2"] }],
    selectedGroup: null, activeTab: "s1", nextId: 3, operations: [],
  };
  return { shop, site, front: SHOP.project, beat: "working", focus: null };
}

const deskOf = (model: FollowModel, paneId: string): "shop" | "site" | null =>
  model.shop.panes.some((row) => row.id === paneId) ? "shop" : model.site.panes.some((row) => row.id === paneId) ? "site" : null;

/** The world after `event`: a focus moves to its pane's project and answers its request, and the first focus back on the lead plays the script. */
function apply(model: FollowModel, event: Parameters<PracticeWorld<FollowModel>["apply"]>[1]): FollowModel | null {
  if (event.kind === "focus_pane") {
    const paneId = String(event.payload.pane_id);
    const desk = deskOf(model, paneId);
    if (!desk) return null;
    const moved = applyDesk(model[desk], event);
    if (!moved) return null;
    const requestId = event.payload.request_id;
    const focus: PaneFocusRequest | null = typeof requestId === "string" ? { request_id: requestId, target_pane_id: paneId, phase: "succeeded", message: null, retryable: false } : model.focus;
    const cameBack = paneId === LEAD && focusedOn(model) !== LEAD;
    return { ...model, [desk]: moved, front: desk === "shop" ? SHOP.project : SITE.project, focus, beat: cameBack ? "answered" : model.beat };
  }
  const front = model.front === SHOP.project ? "shop" : "site";
  const moved = applyDesk(model[front], event);
  return moved ? { ...model, [front]: moved } : null;
}

/** The pane holding the keyboard: the front project's front tab's focused pane. */
export function focusedOn(model: FollowModel): string {
  const desk = model.front === SHOP.project ? model.shop : model.site;
  const front = desk.tabs.find((row) => row.id === desk.activeTab)!;
  return front.focused;
}

function snapshot(model: FollowModel, nowMs: number): WorldSnapshot {
  const agents = rows(model.beat, nowMs);
  const withAgents = (desk: DeskModel): DeskModel => ({ ...desk, panes: desk.panes.map((row) => ({ ...row, agent: agents[row.id] ?? null })) });
  const base = deskSnapshot([{ place: SHOP, model: withAgents(model.shop) }, { place: SITE, model: withAgents(model.site) }], model.front, nowMs);
  const rest = base.rest;
  const navigator = rest.navigator!;
  const lead = agents[LEAD]!;
  // A plain shell the learner opened has no agent, and so no lineage.
  const lineage = (paneId: string) => {
    const row = agents[paneId];
    if (!row?.lineage_parent_pane_id) return undefined;
    const siblings = lead.lineage_child_pane_ids!.map((id) => chip(agents[id]!));
    return [{ pane_id: LEAD, label: lead.identity_label, siblings: [] }, { pane_id: paneId, label: row.identity_label, siblings }];
  };
  const workspaces = navigator.workspaces!.map((workspace) => ({
    ...workspace,
    checkouts: workspace.checkouts.map((checkout) => ({
      ...checkout,
      tabs: checkout.tabs.map((row) => ({ ...row, panes: row.panes.map((one) => ({ ...one, lineage_path: lineage(one.id) })) })),
    })),
  }));
  const scope = { ...navigator.devices![0]!.agent_scope, tasks: tasks(model.beat, nowMs) };
  return {
    ...base,
    rest: {
      ...rest,
      navigator: { ...navigator, workspaces, devices: navigator.devices!.map((device) => ({ ...device, agent_scope: scope })) },
      ui_state: { ...rest.ui_state, sidebar_lens: "tasks" },
      status: { ...rest.status, pane_focus_request: model.focus },
      terminal: { headers: headers(model.beat, nowMs) },
    },
  };
}

const CLAUDE = "\x1b[38;5;209m";
const CODEX = "\x1b[36m";
const DIM = "\x1b[2m";
const BOLD = "\x1b[1m";
const GREEN = "\x1b[32m";
const RESET = "\x1b[0m";
const lines = (...rows: string[]) => rows.join("\r\n");

/** What each agent's terminal shows for `beat`. */
function screen(model: FollowModel, paneId: string): string {
  const answered = model.beat === "answered";
  switch (paneId) {
    case LEAD:
      return lines(
        `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, "",
        `${DIM}>${RESET} Checkout returns a 500 when I pay. Fix it`, "",
        `${CLAUDE}●${RESET} Two parts, one Codex worker each.`,
        `  ${DIM}⎿${RESET}  Find the cause`,
        `  ${DIM}⎿${RESET}  Write a repro test`, "",
        ...(answered
          ? [`${CLAUDE}●${RESET} Saved cards have an empty currency, so the`, "  payment call is refused.", "", `${BOLD}Fill the empty currency for saved cards?${RESET}`, `${CODEX}›${RESET} 1. Yes   2. Show me first`]
          : [`${CLAUDE}✻${RESET} ${DIM}Waiting on 2 workers… (2m 04s)${RESET}`]),
      );
    case CAUSE:
      return lines(
        `${BOLD}>_ Codex${RESET}`, "",
        `${CODEX}›${RESET} Find why checkout returns a 500`, "",
        `${DIM}•${RESET} ${BOLD}Ran${RESET} grep -n currency logs/payment.log`,
        ...(answered ? [`  ${DIM}└${RESET} 214 refused: currency is empty`, "", `${DIM}•${RESET} Saved cards have an empty currency.`, `  ${GREEN}Reported to Fix the checkout 500${RESET}`] : [`  ${DIM}└${RESET} reading…`]),
      );
    case REPRO:
      return lines(
        `${BOLD}>_ Codex${RESET}`, "",
        `${CODEX}›${RESET} Write a test that reproduces the 500`, "",
        `${DIM}•${RESET} ${BOLD}Edited${RESET} test/checkout.test.ts`,
        `  ${GREEN}+ it("pays with a saved card", …)${RESET}`, "",
        `${DIM}•${RESET} Running the test…`,
      );
    case SIGNUP:
      return lines(`${BOLD}>_ Codex${RESET}`, "", `${CODEX}›${RESET} Rewrite the signup page copy`, "", `${DIM}•${RESET} Two headlines are ready.`, "", `${BOLD}Ship the shorter headline?${RESET}`);
    case README:
      return lines(`${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, "", `${DIM}>${RESET} Bring the README up to date`, "", `${CLAUDE}●${RESET} Install steps are current.`);
    default:
      return shellScreen(model.front === SHOP.project ? SHOP : SITE);
  }
}

export const FOLLOW_WORLD: PracticeWorld<FollowModel> = {
  initial,
  apply,
  snapshot,
  panes: (model) => [...model.shop.panes, ...model.site.panes].map((row) => row.id),
  screen: (model, paneId) => screen(model, paneId),
};
