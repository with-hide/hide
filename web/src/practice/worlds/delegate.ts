// Chapter 4's world: a lead agent and two other open agents in one tab, where
// the learner hands work on by saying it. Each request sent from the lead's
// prompt, in whatever words, plays the next part of the script, as the agents
// would, since a practice understands no words: the lead
// starts a child in a tab of its own and later passes it an instruction, the
// child reports back by letter, the lead finds an open agent by what it does
// and writes to it, and a name and ID the lead pane's @ types in (or a sidebar
// row dropped on it, or a menu's copy pasted) names the agent that gets the
// last piece of work.

import { agentReference } from "../../agentReference";
import type { AgentRow, AgentState } from "../../snapshot";
import { PRACTICE_COPY, type PracticeWorld, type WorldSnapshot } from "../world";
import { agent, applyDesk, deskSnapshot, keyText, shellScreen, typed, withLineage, type DeskModel, type PracticePlace } from "./kit";

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
  /** Each request the lead took, in the learner's words, one for each stage played. */
  said: string[];
  /** The last request for the signup agent that named no pane ID, which the lead asks about. */
  unnamed: string | null;
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

const SIGNUP_ROW = resting(working(SIGNUP, "claude", "20m"));

/** The signup agent's reference, as the lead's @, a dropped sidebar row or Copy name and ID types it. */
export const SIGNUP_REFERENCE = agentReference(SIGNUP_ROW);

/** An agent reference (`agentReference.ts`) in a prompt: `@<name> (pane <id> · <task>)`. */
const REFERENCE = /@[^\s(]+ \(pane [^)]*\)/gu;

function initial(): DelegateModel {
  const lead = working(LEAD, "claude", "4m");
  const docs = resting(working(DOCS, "codex", "12m"));
  const signup = SIGNUP_ROW;
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
  return { desk, stage: 0, draft: "", said: [], unnamed: null, copied: null };
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

/**
 * The lead's prompt, sent: any words play the next part of the script, except
 * the last, which takes only a request naming the copied pane ID. An empty
 * prompt sends nothing, and a last request with no ID has the lead ask for one.
 */
function submit(model: DelegateModel): DelegateModel {
  const request = model.draft.trim();
  if (!request) return { ...model, draft: "" };
  const sent = { ...model, draft: "", unnamed: null, said: model.stage < 4 ? [...model.said, request] : model.said };
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
      if (!request.includes(SIGNUP)) return { ...model, draft: "", unnamed: request };
      return { ...sent, stage: 4, desk: withRow(model.desk, SIGNUP, (row) => working(row.pane_id, row.agent_kind, "1s")) };
    default:
      return sent;
  }
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
  const asked = (stage: number) => model.said[stage - 1]!;
  if (model.stage >= 1) rows.push(...said(asked(1)), ...ran("hide agent spawn --parent here …", `Started ${LABELS[CHILD]} in its own tab`), "● It reports back to me by letter when it is done.");
  if (model.stage >= 2) {
    rows.push(...said(asked(2)), ...ran(`hide request send ${CHILD} …`, `Passed on to ${LABELS[CHILD]}`));
    rows.push("", `${OWL}🦉  Letter from ${LABELS[CHILD]}${RESET}`, `  ${GREEN}✓${RESET} The timeout test passes on sqlite. 12 passed.`);
  }
  if (model.stage >= 3) rows.push(...said(asked(3)), ...ran("hide agent list", `${LABELS[DOCS]} · ${LABELS[SIGNUP]} · ${LABELS[CHILD]}`), ...ran(`hide request send ${DOCS} …`, `Letter sent to ${LABELS[DOCS]} (Codex)`));
  if (model.unnamed !== null) rows.push(...said(model.unnamed), "● Which agent should get it? Choose it with the @ on my header.");
  if (model.stage >= 4) rows.push(...said(asked(4)), ...ran(`hide request send ${SIGNUP} …`, `Letter sent to ${SIGNUP} (${LABELS[SIGNUP]})`));
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

function snapshot(model: DelegateModel, nowMs: number): WorldSnapshot {
  const drawn = deskSnapshot([{ place: PLACE, model: model.desk }], PLACE.project, nowMs);
  return model.stage >= 1 ? { ...drawn, rest: withLineage(drawn.rest, LEAD, [CHILD]) } : drawn;
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
    if (event.kind === "key") return p.pane_id === LEAD && typeof p.bytes_base64 === "string" ? typed(model, keyText(p.bytes_base64), submit) : null;
    const desk = applyDesk(model.desk, event);
    return desk ? { ...model, desk } : null;
  },
  snapshot,
  // A reference alone is not the learner's words yet: ⏎ types the step's example in its place, reference and all.
  prompt: (model, paneId) => (paneId === LEAD ? model.draft.replace(REFERENCE, "") : ""),
  panes: (model) => model.desk.panes.map((pane) => pane.id),
  // A pane the learner opened, a new tab or a split, is a shell with no agent in it.
  screen: (model, paneId, cols) => SCREENS[paneId]?.(model, cols) ?? shellScreen(PLACE),
};
