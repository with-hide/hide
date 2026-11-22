// Flow 10's world: one invented project with an agent that asks which way to
// go, and the learner's phone beside the screen, open on that agent's
// conversation. The phone's choice is answered as the computer answers it:
// the agent's own screen takes the keys for the choice (↓ to its row, then
// Enter) and the agent goes on, running a command the phone counts by the
// clock. A practice screen has no phone and no relay, so the phone is drawn
// beside the screen from the phone's own parts (`PracticePhone.tsx`). Nothing
// leaves the page.

import type { PhoneAsk } from "../../mobile/protocol";
import type { PaneHeader } from "../../snapshot";
import type { CoreEvent, PracticePhoneScreen, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, counted, deskSnapshot, oneTabDesk, rowRequest, rowState, type DeskModel, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "fix/review" };
/** The pane of the agent that asks. */
export const ASKING = "review";
/** How long after the answer the agent's screen has taken the keys and it goes on. */
const MOVES_AFTER_MS = 1_800;
const MINUTE_MS = 60_000;

const TITLE = "Review fixes";
const READ = "I read the 4 review comments.";
const QUESTION = "How should I apply them?";
const CHOICES = [
  { id: "1", label: "All in this PR", note: "One PR to review" },
  { id: "2", label: "Split into two PRs", note: "The test fixes go first" },
  { id: "3", label: "Only the important ones", note: null },
] as const;
const NEXT = "Splitting them. Running the tests first.";
const COMMAND = "pnpm test";

/**
 * What the world can change: the desk, when the agent began to ask, the
 * choice marked on the phone, the choice sent and when, and when the agent
 * went on after it.
 */
export type AnswerModel = { desk: DeskModel; askedAt: number; picked: string | null; answered: { choice: string; at: number } | null; movedAt: number | null };

/** Whether a choice is marked on the phone. */
export const picked = (model: AnswerModel) => model.picked !== null;
/** Whether a choice was sent with 답하기. */
export const answered = (model: AnswerModel) => model.answered !== null;
/** Whether the agent's screen took the keys and the agent went on. */
export const moved = (model: AnswerModel) => model.movedAt !== null;

function initial(): AnswerModel {
  const askedAt = Date.now() - 3 * MINUTE_MS;
  return { desk: oneTabDesk({ id: ASKING, agent: null, title: TITLE }), askedAt, picked: null, answered: null, movedAt: null };
}

const choiceOf = (payload: Record<string, unknown>) => CHOICES.find((choice) => choice.id === payload.choice)?.id ?? null;

function apply(model: AnswerModel, event: CoreEvent): AnswerModel | null {
  switch (event.kind) {
    // A press on the phone marks a choice; nothing leaves the phone yet.
    case "practice_phone_pick": {
      const choice = choiceOf(event.payload);
      return choice && !answered(model) && choice !== model.picked ? { ...model, picked: choice } : null;
    }
    // 답하기: the computer types the choice's keys into the agent's screen.
    case "practice_phone_answer": {
      const choice = choiceOf(event.payload);
      return choice && !answered(model) ? { ...model, picked: choice, answered: { choice, at: Date.now() } } : null;
    }
    // The world's own beat: the agent's screen took the keys, and the agent goes on.
    case "practice_agent_moves":
      return answered(model) && !moved(model) ? { ...model, movedAt: Date.now() } : null;
    default: {
      const desk = applyDesk(model.desk, event);
      return desk ? { ...model, desk } : null;
    }
  }
}

/** The agent's row: asking in 내 차례 until it goes on, then at work. */
function deskOf(model: AnswerModel): DeskModel {
  const working = moved(model);
  const row = agent({
    pane_id: ASKING, identity_label: TITLE, agent_kind: "claude",
    group: working ? "working" : "needs_you", mark: working ? "going" : "turn", status_code: working ? "working" : "question", demand: working ? "none" : "question",
    activity: working ? "working" : "idle", unread: !working, elapsed: working ? "0s" : "3m",
    request: rowRequest(TITLE, TITLE, model.askedAt, null),
    state: working
      ? rowState("working", true, model.movedAt!)
      : rowState("asking", true, model.askedAt, { ask: { verb: "answer", what: QUESTION, more: 0 }, line: { text: QUESTION, mode: "request", tone: { kind: "warning", read: false } } }),
  });
  return { ...model.desk, panes: model.desk.panes.map((pane) => (pane.id === ASKING ? { ...pane, agent: row } : pane)) };
}

function snapshot(model: AnswerModel, nowMs: number): WorldSnapshot {
  const drawn = deskSnapshot([{ place: PLACE, model: deskOf(model) }], PLACE.project, nowMs);
  const rest = moved(model) ? drawn.rest : counted(drawn.rest, "needs_you");
  const header: PaneHeader = moved(model)
    ? { working: true, notice: null, band: null, request: { text: TITLE, images: 0 }, progress: `Running ${COMMAND}` }
    : { working: false, notice: null, band: { kind: "answer", reason: QUESTION, since_unix_ms: model.askedAt, read: false, action: null, more: 0 }, request: { text: TITLE, images: 0 } };
  return { ...drawn, rest: { ...rest, terminal: { headers: { [ASKING]: header } } } };
}

/** The phone's detail of the agent: its conversation, its ask while it asks, and the command it runs once it went on. */
function phone(model: AnswerModel): PracticePhoneScreen {
  const ask: PhoneAsk = {
    kind: "question", turn: String(model.askedAt), since_ms: model.askedAt, text: QUESTION, truncated: false, answerable: true,
    choices: CHOICES.map((choice) => ({ id: choice.id, label: choice.label, ...(choice.note ? { note: choice.note } : {}) })),
  };
  const sent = model.answered ? CHOICES.find((choice) => choice.id === model.answered!.choice)!.label : null;
  return {
    pane: ASKING, title: TITLE, kind: "claude",
    said: moved(model) ? [{ who: "agent", text: READ }, { who: "you", text: sent! }, { who: "agent", text: NEXT }] : [{ who: "agent", text: READ }],
    ask: moved(model) ? null : ask,
    picked: model.picked,
    answered: model.answered !== null,
    running: model.movedAt,
  };
}

const CLAUDE = "\x1b[38;2;215;119;87m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const RESET = "\x1b[0m";

/** The agent's own screen: its question with the highlighted row, which the computer's keys move and take. */
function screen(model: AnswerModel, _paneId: string, cols: number): string {
  const head = ["", `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`, "", `${DIM}>${RESET} Apply the review comments on the PR`, "", `${CLAUDE}●${RESET} ${READ}`, ""];
  if (moved(model)) {
    const label = CHOICES.find((choice) => choice.id === model.answered!.choice)!.label;
    return [...head, ` ${DIM}✔${RESET} ${label}`, "", `${CLAUDE}●${RESET} ${NEXT}`, "", `${CLAUDE}●${RESET} ${BOLD}Bash${RESET}(${COMMAND})`, `  ${DIM}⎿  Running…${RESET}`].join("\r\n");
  }
  // Answered: the keys took the highlight to the chosen row; until then it stands on the first.
  const at = model.answered ? model.answered.choice : "1";
  const rows = CHOICES.map((choice, index) => (choice.id === at ? ` ${CLAUDE}❯ ${index + 1}. ${choice.label}${RESET}` : `   ${index + 1}. ${choice.label}`));
  return [...head, `${DIM}${"─".repeat(cols)}${RESET}`, ` ${BOLD}${QUESTION}${RESET}`, "", ...rows, "", ` ${DIM}Enter to select · ↑/↓ to navigate · Esc to cancel${RESET}`].join("\r\n");
}

export const ANSWER_WORLD: PracticeWorld<AnswerModel> = {
  initial,
  apply,
  snapshot,
  panes: (model) => deskOf(model).panes.map((pane) => pane.id),
  screen,
  phone,
  // The agent's screen takes the keys a moment after the answer, as a real one does, and the agent goes on.
  later: (model) => (model.answered && !moved(model) ? { at: model.answered.at + MOVES_AFTER_MS, event: { kind: "practice_agent_moves", payload: {} } } : null),
};
