// Chapter 4's world: a lead agent and two other open agents in one tab, where
// the learner hands work on by saying it. Each request typed into the lead's
// prompt plays the next part of the script, as the agents would: the lead
// starts a child in a tab of its own and later passes it an instruction, the
// child reports back by letter, the lead finds an open agent by what it does
// and writes to it, and a pane ID copied from a pane's menu names the agent
// that gets the last piece of work.

import type { AgentScope, AgentTreeScope } from "../../agentScope";
import type { AgentRow, AgentState, SnapshotRest } from "../../snapshot";
import { PRACTICE_COPY, type PracticeWorld, type WorldSnapshot } from "../world";
import { agent, applyDesk, deskSnapshot, type DeskModel, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };

/** The panes, named the way Herdr names them, since a copied pane ID is one of these. */
export const LEAD = "w1:p1";
export const DOCS = "w1:p2";
export const SIGNUP = "w1:p3";
export const CHILD = "w1:p4";

const CHILD_TAB = "t2";

/**
 * How far the script has played: 0 before any request, then one more for each
 * request the lead took: the child started, the child told and reported back,
 * the docs writer written to, and the signup agent given work by its pane ID.
 */
export type DelegateStage = 0 | 1 | 2 | 3 | 4;

export type DelegateModel = {
  desk: DeskModel;
  stage: DelegateStage;
  /** What the learner has typed into the lead's prompt and not sent yet. */
  draft: string;
  /** What the practice screen copied last; it never reaches the operator's clipboard. */
  copied: string | null;
};

const LABELS: Record<string, string> = {
  [LEAD]: "Fix the checkout timeout",
  [DOCS]: "Update the API docs",
  [SIGNUP]: "Signup copy",
  [CHILD]: "Repro the timeout test",
};

/** A row the core shows as Seen: finished and read, resting. */
function resting(row: AgentRow): AgentRow {
  const state: AgentState = {
    ...row.state,
    working: false, chip_tone: { kind: "subtle", read: true }, mark_tone: { kind: "subtle", read: true },
    bucket: "resting", attention_rank: 4, search_tone: "muted", subtree: "quiet", link: "idle", link_rank: 2, verb: "idle", tree_rank: 4,
    session: { group: "idle", line: null, unfinished: false },
  };
  return { ...row, state, group: "seen", mark: "quiet", status_code: "idle", activity: "idle", emphasized: false };
}

const working = (pane_id: string, agent_kind: string, elapsed: string) =>
  agent({ pane_id, identity_label: LABELS[pane_id]!, agent_kind, group: "working", mark: "going", status_code: "working", activity: "working", elapsed });

function initial(): DelegateModel {
  const lead = working(LEAD, "claude", "4m");
  const docs = resting(working(DOCS, "codex", "12m"));
  const signup = resting(working(SIGNUP, "claude", "20m"));
  const pane = (row: AgentRow) => ({ id: row.pane_id, agent: row, title: row.identity_label });
  const desk: DeskModel = {
    panes: [pane(lead), pane(docs), pane(signup)],
    tabs: [{
      id: "t1", name: null, focused: LEAD, zoomed: false,
      root: { type: "split", direction: "right", ratio: 0.56, first: { type: "pane", pane_id: LEAD }, second: { type: "split", direction: "down", ratio: 0.5, first: { type: "pane", pane_id: DOCS }, second: { type: "pane", pane_id: SIGNUP } } },
    }],
    groups: [{ id: null, name: null, tabs: ["t1"] }],
    selectedGroup: null,
    activeTab: "t1",
    nextId: 3,
    operations: [],
  };
  return { desk, stage: 0, draft: "", copied: null };
}

/** The desk with the agent row of `paneId` replaced. */
function withRow(desk: DeskModel, paneId: string, change: (row: AgentRow) => AgentRow): DeskModel {
  return { ...desk, panes: desk.panes.map((pane) => (pane.id === paneId && pane.agent ? { ...pane, agent: change(pane.agent) } : pane)) };
}

/** The lead starts a Codex child; Hide gives it a tab of its own and leaves the screen where it was. */
function spawnChild(desk: DeskModel): DeskModel {
  const child: AgentRow = {
    ...working(CHILD, "codex", "1s"),
    delegated: true, lineage_parent_pane_id: LEAD, lineage_path_pane_ids: [LEAD], lineage_depth: 1,
  };
  child.state = { ...child.state, root: false };
  const withChild: DeskModel = {
    ...desk,
    panes: [...desk.panes, { id: CHILD, agent: child, title: child.identity_label }],
    tabs: [...desk.tabs, { id: CHILD_TAB, name: null, root: { type: "pane", pane_id: CHILD }, focused: CHILD, zoomed: false }],
    groups: desk.groups.map((group) => (group.id === null ? { ...group, tabs: [...group.tabs, CHILD_TAB] } : group)),
  };
  return withRow(withChild, LEAD, (lead) => ({ ...lead, lineage_child_pane_ids: [CHILD], lineage_collapsed: false, direct_child_counts: { error: 0, approval: 0, question: 0, working: 1, done: 0 }, descendant_counts: { error: 0, approval: 0, question: 0, working: 1, done: 0 } }));
}

const NO_COUNTS = { error: 0, approval: 0, question: 0, working: 0, done: 0 };

/** The lead's request, sent: the next part of the script, or nothing where the request does not fit it. */
function submit(model: DelegateModel): DelegateModel {
  const sent = { ...model, draft: "" };
  switch (model.stage) {
    case 0:
      return { ...sent, stage: 1, desk: spawnChild(model.desk) };
    case 1: {
      // The child takes the instruction, finishes, and reports by letter; an ordinary child rests as Seen.
      const desk = withRow(model.desk, CHILD, resting);
      return { ...sent, stage: 2, desk: withRow(desk, LEAD, (lead) => ({ ...lead, direct_child_counts: NO_COUNTS, descendant_counts: NO_COUNTS })) };
    }
    case 2:
      return { ...sent, stage: 3, desk: withRow(model.desk, DOCS, (row) => working(row.pane_id, row.agent_kind, "1s")) };
    case 3:
      // Only the copied pane ID says which agent; a request that names none waits for it.
      if (!model.draft.includes(SIGNUP)) return sent;
      return { ...sent, stage: 4, desk: withRow(model.desk, SIGNUP, (row) => working(row.pane_id, row.agent_kind, "1s")) };
    default:
      return sent;
  }
}

const decode = (base64: string) => new TextDecoder().decode(Uint8Array.from(atob(base64), (char) => char.charCodeAt(0)));

/** The lead's prompt after the keys `text`: ⌃U clears it, ⌫ takes a character back, ⏎ sends it; an arrow or other escape sequence moves nothing. */
function typed(model: DelegateModel, text: string): DelegateModel {
  let next = model;
  let escape = false;
  for (const char of text) {
    if (escape) escape = !/[A-Za-z~]/.test(char);
    else if (char === "\x1b") escape = true;
    else if (char === "\r") next = submit(next);
    else if (char === "\x15") next = { ...next, draft: "" };
    else if (char === "\x7f") next = { ...next, draft: [...next.draft].slice(0, -1).join("") };
    else if (char >= " ") next = { ...next, draft: next.draft + char };
  }
  return next;
}

// --- What each terminal shows ------------------------------------------------

const CLAUDE = "\x1b[38;2;215;119;87m";
const CODEX = "\x1b[36m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const GREEN = "\x1b[32m";
const OWL = "\x1b[33m";
const RESET = "\x1b[0m";
const lines = (...rows: string[]) => rows.join("\r\n");
const rule = (cols: number) => `${DIM}${"─".repeat(cols)}${RESET}`;

/** The lead's conversation: what it had done, then each request it took and what it did. */
function leadScreen(model: DelegateModel, cols: number): string {
  const said = (request: string) => ["", `${DIM}>${RESET} ${request}`, ""];
  const ran = (command: string, result: string) => [`● ${BOLD}Bash${RESET}(${command})`, `  ${DIM}⎿${RESET}  ${result}`];
  // A working agent's pane wears a band over its first rows.
  const rows = [
    "",
    "",
    `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`,
    `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`,
    ...said("Checkout gives up after 3 seconds. Make it 10"),
    "● Raised the checkout timeout from 3s to 10s.",
    `  ${DIM}⎿${RESET}  src/payment/checkout.ts`,
  ];
  if (model.stage >= 1) rows.push(...said("Hand the tests to a Codex child"), ...ran("hide agent spawn --parent here …", `Started ${LABELS[CHILD]} in its own tab`), "● It reports back to me by letter when it is done.");
  if (model.stage >= 2) {
    rows.push(...said("Tell the child to use sqlite"), ...ran(`hide request send ${CHILD} …`, `Passed on to ${LABELS[CHILD]}`));
    rows.push("", `${OWL}🦉  Letter from ${LABELS[CHILD]}${RESET}`, `  ${GREEN}✓${RESET} The timeout test passes on sqlite. 12 passed.`);
  }
  if (model.stage >= 3) rows.push(...said("Tell the docs writer the timeout changed"), ...ran("hide agent list", `${LABELS[DOCS]} · ${LABELS[SIGNUP]} · ${LABELS[CHILD]}`), ...ran(`hide request send ${DOCS} …`, `Letter sent to ${LABELS[DOCS]} (Codex)`));
  if (model.stage >= 4) rows.push(...said(`${SIGNUP} update the signup copy for the new timeout`), ...ran(`hide request send ${SIGNUP} …`, `Letter sent to ${SIGNUP} (${LABELS[SIGNUP]})`));
  if (model.stage === 1) rows.push("", `${CLAUDE}✻ Waiting on 1 child… ${DIM}(esc to interrupt)${RESET}`);
  rows.push("", rule(cols), `${DIM}>${RESET} ${model.draft}`, rule(cols));
  return lines(...rows);
}

function codexHead(): string[] {
  return [
    `${DIM}╭──────────────────────────────╮${RESET}`,
    `${DIM}│${RESET} ${BOLD}>_ Codex${RESET}                     ${DIM}│${RESET}`,
    `${DIM}│${RESET} ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}        ${DIM}│${RESET}`,
    `${DIM}╰──────────────────────────────╯${RESET}`,
    "",
  ];
}

/** A letter as the receiving agent sees it: the prompt Hide wakes it with. */
const letter = (prompt: string, body: string) => [`${prompt} ${OWL}🦉  Letter from ${LABELS[LEAD]}${RESET}`, `  ${body}`, ""];

function docsScreen(model: DelegateModel): string {
  const rows = [...codexHead(), `${CODEX}›${RESET} Update the API docs for checkout`, "", `${DIM}•${RESET} ${BOLD}Edited${RESET} docs/api/checkout.md`, `  ${DIM}└${RESET} 3 sections`, ""];
  if (model.stage >= 3) rows.push(...letter(`${CODEX}›${RESET}`, "The checkout timeout is now 10 seconds."), `${DIM}•${RESET} Updating docs/api/checkout.md: timeout 3s → 10s`);
  return lines(...rows);
}

function signupScreen(model: DelegateModel): string {
  const rows = [`${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`, "", `${DIM}>${RESET} Write the signup page copy`, "", "● Wrote the copy in src/signup/strings.ts.", ""];
  if (model.stage >= 4) rows.push(...letter(`${DIM}>${RESET}`, "Update the signup copy for the new timeout."), "● Changing the wait message to “up to 10 seconds”.");
  return lines(...rows);
}

function childScreen(model: DelegateModel): string {
  const rows = [...codexHead(), `${CODEX}›${RESET} Write a test that reproduces the checkout timeout`, "", `${DIM}•${RESET} ${BOLD}Added${RESET} tests/checkout-timeout.test.ts`, ""];
  if (model.stage >= 2) rows.push(...letter(`${CODEX}›${RESET}`, "Use sqlite for the test database."), `${DIM}•${RESET} ${BOLD}Ran${RESET} pnpm test checkout`, `  ${DIM}└${RESET} ${GREEN}12 passed${RESET}`, "", `${DIM}•${RESET} Reported to ${LABELS[LEAD]}`);
  return lines(...rows);
}

// --- The snapshot ---------------------------------------------------------------

/** Every agent scope in `rest`, with the child drawn one level under the lead and the lead no longer its only root. */
function withLineage(rest: SnapshotRest): SnapshotRest {
  const nest = (tree: AgentTreeScope): AgentTreeScope => {
    const rows = tree.rows.filter((row) => row.pane_id !== CHILD).flatMap((row) => (row.pane_id === LEAD ? [row, { pane_id: CHILD, occurrence: 0, depth: 1 }] : [row]));
    return { ...tree, rows, visible_rows: rows };
  };
  const scope = (one: AgentScope): AgentScope => ({
    ...one,
    roots: one.roots.filter((row) => row.pane_id !== CHILD),
    children: { [LEAD]: [CHILD] },
    descendants: { [LEAD]: 1 },
    tree: nest(one.tree),
    sidebar_tree: nest(one.sidebar_tree),
    global_tree: nest(one.global_tree),
  });
  const navigator = rest.navigator!;
  return {
    ...rest,
    navigator: {
      ...navigator,
      agent_scope: navigator.agent_scope && scope(navigator.agent_scope),
      devices: navigator.devices?.map((device) => ({ ...device, agent_scope: scope(device.agent_scope) })),
      workspaces: navigator.workspaces?.map((workspace) => ({
        ...workspace,
        agent_scope: scope(workspace.agent_scope),
        checkouts: workspace.checkouts.map((checkout) => ({
          ...checkout,
          agent_scope: scope(checkout.agent_scope),
          tabs: checkout.tabs.map((tab) => (tab.id === CHILD_TAB ? { ...tab, delegated: true } : tab)),
        })),
      })),
    },
  };
}

function snapshot(model: DelegateModel, nowMs: number): WorldSnapshot {
  const drawn = deskSnapshot([{ place: PLACE, model: model.desk }], PLACE.project, nowMs);
  return model.stage >= 1 ? { ...drawn, rest: withLineage(drawn.rest) } : drawn;
}

const SCREENS: Record<string, (model: DelegateModel, cols: number) => string> = {
  [LEAD]: leadScreen,
  [DOCS]: docsScreen,
  [SIGNUP]: signupScreen,
  [CHILD]: childScreen,
};

export const DELEGATE_WORLD: PracticeWorld<DelegateModel> = {
  initial,
  apply(model, event) {
    const p = event.payload;
    if (event.kind === PRACTICE_COPY) return { ...model, copied: String(p.text) };
    // Only the lead is talked to here; keys typed into another pane reach no one.
    if (event.kind === "key") return p.pane_id === LEAD && typeof p.bytes_base64 === "string" ? typed(model, decode(p.bytes_base64)) : null;
    const desk = applyDesk(model.desk, event);
    return desk ? { ...model, desk } : null;
  },
  snapshot,
  panes: (model) => model.desk.panes.map((pane) => pane.id),
  screen: (model, paneId, cols) => SCREENS[paneId]!(model, cols),
  lane: false,
};
