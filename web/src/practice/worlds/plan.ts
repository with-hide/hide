// Chapter 7's world: one Claude agent the learner asks for a bigger change,
// plan first. Whatever words the learner sends play the next part of the
// script, since a practice understands no words: the agent puts up a plan
// for review (`hide plan set --ask`), which the core lays on its row as a
// question, and the next words are the review's answer, so the agent approves
// its plan (`hide plan approve`) and starts on the first step. Approval is a
// conversation, as in Hide: no event of the shell's approves a plan.

import type { AgentRow, AgentState, AskSubject, DetailItem, PaneHeader, PlanStep, PlanView, StatusTone, WorkSession } from "../../snapshot";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, counted, deskSnapshot, keyText, oneTabDesk, shellScreen, typed, type DeskModel, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };

export const LEAD = "lead";
const LABEL = "Payment retries";
const SESSION: WorkSession = { device_id: "local", agent: "claude", session_id: "practice-plan" };

/** 0 before the request, 1 once the plan waits for review, 2 once it is approved and the first step runs. */
export type PlanStage = 0 | 1 | 2;

export type PlanModel = {
  desk: DeskModel;
  stage: PlanStage;
  /** What the learner has typed into the agent's prompt and not sent yet. */
  draft: string;
  /** Each request the agent took, in the learner's words. */
  said: string[];
  /** When the plan was put up and last changed. */
  createdAt: number;
  updatedAt: number;
  /** The 보기 slot: what it opened, oldest first, and whether the lane draws it. */
  detail: DetailItem[];
  detailShown: boolean;
  /** The core's count of lane calls; an open in the 보기 slot is one, which brings an unpinned lane up. */
  laneCalls: number;
};

/** Whether the 보기 slot shows this chapter's plan in front. */
export function planInSlot(model: PlanModel): boolean {
  const top = model.detail.at(-1);
  return model.detailShown && top?.kind === "plan" && top.session.session_id === SESSION.session_id;
}

const tone = (kind: StatusTone["kind"]): StatusTone => ({ kind, read: false });

/** The agent's row at rest after its first answer, as the core shows a finished, read root: Seen. */
function resting(row: AgentRow): AgentRow {
  const state: AgentState = {
    ...row.state,
    working: false, chip_tone: { kind: "subtle", read: true }, mark_tone: { kind: "subtle", read: true },
    bucket: "resting", attention_rank: 4, search_tone: "muted", subtree: "quiet", link: "idle", link_rank: 2, verb: "idle", tree_rank: 3,
    session: { group: "idle", line: null, unfinished: false },
  };
  return { ...row, state, group: "seen", mark: "quiet", status_code: "idle", activity: "idle", emphasized: false };
}

/**
 * The resting row with the plan's review laid on it, as the core lays it
 * (`runtime/plans.rs`, `lay_plan_reviews`): a question with no words, only its
 * subject, which the row's line, its ask and the pane band carry, so the row
 * waits on the operator in Needs You.
 */
function reviewing(row: AgentRow, subject: AskSubject, since: number): AgentRow {
  const state: AgentState = {
    ...row.state,
    attention: true, needs_you: true, title_emphasized: true, asking: true, working: false,
    chip_tone: tone("warning"), mark_tone: tone("warning"), bucket: "turn", attention_rank: 1, link_rank: 0, search_tone: "attention", subtree: "waiting", link: "question",
    verb: "answer", request_todo: true, tree_rank: 0, request_since: since,
    session: { group: "needs_you", line: null, unfinished: false },
    ask: { verb: "answer", what: null, more: 0, subject },
    line: { text: "", mode: "request", tone: tone("warning"), subject },
  };
  return {
    ...row, state, group: "needs_you", mark: "turn", status_code: "question", demand: "question", activity: "idle", emphasized: true,
    user_turn: { kind: "question", content: null }, ask_subject: subject, changed_at_unix_ms: since,
  };
}

function initial(): PlanModel {
  const lead = resting(agent({ pane_id: LEAD, identity_label: LABEL, agent_kind: "claude", group: "seen", mark: "quiet", status_code: "idle", elapsed: "6m", session_id: SESSION.session_id }));
  const now = Date.now();
  return { desk: oneTabDesk({ id: LEAD, agent: lead, title: LABEL }), stage: 0, draft: "", said: [], createdAt: now, updatedAt: now, detail: [], detailShown: false, laneCalls: 0 };
}

// --- The plan ------------------------------------------------------------------

const STEPS: { key: string; title: string; depends_on: string[]; output: PlanStep["output"] }[] = [
  { key: "config", title: "Read the retry count from settings", depends_on: [], output: null },
  { key: "backoff", title: "Back off exponentially between retries", depends_on: ["config"], output: null },
  { key: "pr", title: "Test it and open a PR", depends_on: ["backoff"], output: "pr" },
];

const FIRST_LINE = "Reading src/payment/retry.ts";

/** The plan as the core's `PlanView` draws it: all planned while it waits for review, the first step at work once approved. */
function planView(model: PlanModel): PlanView | null {
  if (model.stage === 0) return null;
  const running = model.stage === 2;
  const steps: PlanStep[] = STEPS.map((step, index) => ({
    ...step,
    status: running && index === 0 ? "working" : "planned",
    line: running && index === 0 ? FIRST_LINE : null,
    changed: false, change: null,
    // A step waits while a step it waits on is neither done nor dropped.
    waiting: step.depends_on.length > 0,
  }));
  return {
    pane_id: LEAD,
    session: SESSION,
    steps,
    review: running ? "approved" : "proposed",
    ask: running ? null : "Check where the retry count lives and how long the backoff may grow",
    progress: { current: "config", next: "backoff", done: 0, total: steps.length, finished: false },
    created_at_unix_ms: model.createdAt,
    updated_at_unix_ms: model.updatedAt,
  };
}

const SUBJECT: AskSubject = { cause: "plan_review", steps: STEPS.length };

// --- The agent's prompt ----------------------------------------------------------

/**
 * The prompt, sent: the first words are the request the agent plans before
 * doing, the next the review's answer, which it approves and starts on; words
 * after that it takes on while it works. An empty prompt sends nothing.
 */
function submit(model: PlanModel): PlanModel {
  const request = model.draft.trim();
  if (!request) return { ...model, draft: "" };
  const sent = { ...model, draft: "", said: [...model.said, request] };
  const now = Date.now();
  const row = (change: (one: AgentRow) => AgentRow): DeskModel => ({ ...model.desk, panes: model.desk.panes.map((pane) => (pane.agent ? { ...pane, agent: change(pane.agent) } : pane)) });
  switch (model.stage) {
    case 0:
      return { ...sent, stage: 1, createdAt: now, updatedAt: now, desk: row((one) => reviewing(one, SUBJECT, now)) };
    case 1:
      // `hide plan approve` lifts the question; the agent is at work on the first step.
      return {
        ...sent, stage: 2, updatedAt: now,
        desk: row(() => agent({ pane_id: LEAD, identity_label: LABEL, agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "1s", session_id: SESSION.session_id })),
      };
    default:
      return sent;
  }
}

// --- The events --------------------------------------------------------------------

function apply(model: PlanModel, event: CoreEvent): PlanModel | null {
  const p = event.payload;
  switch (event.kind) {
    // Only the agent is talked to here; keys typed into another pane reach no one.
    case "key":
      return p.pane_id === LEAD && typeof p.bytes_base64 === "string" ? typed(model, keyText(p.bytes_base64), submit) : null;
    // The plan opens in the 보기 slot as a lane call, the way the core numbers one; a record or a card this world keeps none of.
    case "detail_open": {
      const item = p.item as DetailItem;
      if (item.kind !== "plan" || item.session.session_id !== SESSION.session_id || model.stage === 0) return null;
      return { ...model, detail: [...model.detail.filter((one) => one.kind !== "plan"), item], detailShown: true, laneCalls: model.laneCalls + 1 };
    }
    case "detail_back":
      return model.detail.length > 1 ? { ...model, detail: model.detail.slice(0, -1) } : null;
    case "detail_close":
      return model.detail.length ? { ...model, detail: [], detailShown: false } : null;
    case "detail_show":
      return model.detail.length ? { ...model, detailShown: p.shown === true } : null;
    default: {
      const desk = applyDesk(model.desk, event);
      return desk ? { ...model, desk } : null;
    }
  }
}

// --- The snapshot --------------------------------------------------------------------

/** The agent pane's head as the core sends it: at rest, asking for the review on its second row, or at work on the request. */
function header(model: PlanModel): PaneHeader {
  if (model.stage === 1) return { working: false, notice: null, band: { kind: "answer", reason: null, since_unix_ms: model.createdAt, read: false, action: null, more: 0, subject: SUBJECT } };
  if (model.stage === 2) return { working: true, notice: null, band: null, request: { text: model.said.at(-1)!, images: 0 } };
  return { working: false, notice: null, band: null };
}

function snapshot(model: PlanModel, nowMs: number): WorldSnapshot {
  const drawn = deskSnapshot([{ place: PLACE, model: model.desk }], PLACE.project, nowMs);
  const base = model.stage === 2 ? drawn.rest : counted(drawn.rest, model.stage === 1 ? "needs_you" : "seen");
  const plan = planView(model);
  const shown = model.detailShown && model.detail.length > 0;
  return {
    ...drawn,
    rest: {
      ...base,
      plans: { items: plan ? [plan] : [] },
      ui_state: { ...base.ui_state!, detail: { items: model.detail, shown } },
      // Opening the 보기 slot opens the lane, as the core does on `detail_open`.
      workspace_view: { ...base.workspace_view!, open: shown, lane_called: model.laneCalls, lane_calls: model.laneCalls, lane_called_side: "view" },
      terminal: { headers: { [LEAD]: header(model) } },
    },
  };
}

// --- What the agent's terminal shows ---------------------------------------------------

const CLAUDE = "\x1b[38;2;215;119;87m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const RESET = "\x1b[0m";
const rule = (cols: number) => `${DIM}${"─".repeat(cols)}${RESET}`;

/** The agent's conversation: the question it answered before, the plan it put up, and the start on its first step. */
function leadScreen(model: PlanModel, cols: number): string {
  const said = (request: string) => ["", `${DIM}>${RESET} ${request}`, ""];
  const ran = (command: string, result: string) => [`● ${BOLD}Bash${RESET}(${command})`, `  ${DIM}⎿${RESET}  ${result}`];
  const rows = [
    "",
    "",
    `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`,
    `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`,
    ...said("Where do payment retries happen?"),
    "● In src/payment/retry.ts: three tries, one second apart.",
  ];
  if (model.stage >= 1) {
    rows.push(
      ...said(model.said[0]!),
      ...ran("hide plan set --ask …", `Plan put up for review · ${STEPS.length} steps`),
      "● Here is the plan before I change anything:",
      ...STEPS.map((step, index) => `  ${index + 1}. ${step.title}`),
      "  Tell me to go, or what to change.",
    );
  }
  if (model.stage >= 2) {
    rows.push(...said(model.said[1]!), ...ran("hide plan approve", "Plan approved"), ...ran(`hide plan node ${STEPS[0]!.key} --status working`, STEPS[0]!.title));
    for (const later of model.said.slice(2)) rows.push(...said(later), "● Noted. I'll fold that into the plan as I go.");
    rows.push("", `${CLAUDE}✻ ${FIRST_LINE}… ${DIM}(esc to interrupt)${RESET}`);
  }
  rows.push("", rule(cols), `${DIM}>${RESET} ${model.draft}`, rule(cols));
  return rows.join("\r\n");
}

export const PLAN_WORLD: PracticeWorld<PlanModel> = {
  initial,
  apply,
  snapshot,
  prompt: (model, paneId) => (paneId === LEAD ? model.draft : ""),
  panes: (model) => model.desk.panes.map((pane) => pane.id),
  // A pane the learner opened, a new tab or a split, is a shell with no agent in it.
  screen: (model, paneId, cols) => (paneId === LEAD ? leadScreen(model, cols) : shellScreen(PLACE)),
};
