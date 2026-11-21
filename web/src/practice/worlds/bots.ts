// Flow 6's world: one invented project with the learner's agent at work, a
// bot already connected as 전체, and a second bot's request waiting. A program
// outside Hide asks with `hide access request`, which a practice screen cannot
// run, so the request is staged: it waits in Settings › 봇 from the start, as
// it would once the program asked. The world answers the approval as the core
// does (`access_approve`, `access_mode`): the request leaves, the bot is
// connected in the mode picked, and its owl lands on the rail. A few seconds
// later the 전체 bot's agent asks the operator a question, and its row stands in
// 내 차례 wearing that bot's owl. Nothing leaves the page.

import type { BotModeKind } from "../../bots";
import type { AccessConnection, AccessSection, AgentRow, PaneHeader } from "../../snapshot";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, counted, deskSnapshot, oneTabDesk, rowRequest, rowState, type DeskModel, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };
const SECOND_MS = 1_000;
const MINUTE_MS = 60 * SECOND_MS;

/** The bot that asks to connect, staged, and the 전체 bot already connected. */
export const ASKING = { request: "req-nightly", connection: "conn-nightly", name: "nightly", program: "/usr/local/bin/nightly" } as const;
export const FULL = { connection: "conn-hermes", name: "hermes", program: "/usr/local/bin/hermes" } as const;
/** The pane of the agent the 전체 bot started, which asks once the learner approved the request. */
export const BOT_PANE = "deploy";
/** How long after the approval the 전체 bot's agent asks. */
const ASKS_AFTER_MS = 2_500;

/**
 * What the world can change: the desk, the mode each connected bot works in
 * (the asking bot is absent until approved), when the request was approved,
 * and whether the 전체 bot's agent has asked.
 */
export type BotsModel = { desk: DeskModel; modes: Record<string, BotModeKind>; approvedAt: number | null; asked: boolean };

/** Whether the learner approved the waiting request, in either mode. */
export const approved = (model: BotsModel) => model.approvedAt !== null;
/** Whether the 전체 bot's agent is waiting on the learner in 내 차례. */
export const botAsks = (model: BotsModel) => model.asked;

const TITLE = { lead: "Fix the checkout 500", deploy: "Deploy the release" };
const QUESTION = "Push the tag now?";

function initial(): BotsModel {
  const lead = agent({ pane_id: "lead", identity_label: TITLE.lead, agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "4m" });
  return { desk: oneTabDesk({ id: "lead", agent: lead, title: TITLE.lead }), modes: { [FULL.connection]: "all" }, approvedAt: null, asked: false };
}

const modeOf = (payload: Record<string, unknown>): BotModeKind | null => {
  const kind = (payload.mode as { kind?: unknown } | undefined)?.kind;
  return kind === "own" || kind === "all" ? kind : null;
};

function apply(model: BotsModel, event: CoreEvent): BotsModel | null {
  const p = event.payload;
  switch (event.kind) {
    // The request approved in the mode picked: the bot is connected from then on.
    case "access_approve": {
      const mode = modeOf(p);
      if (p.id !== ASKING.request || !mode || approved(model)) return null;
      return { ...model, modes: { ...model.modes, [ASKING.connection]: mode }, approvedAt: Date.now() };
    }
    // 모드 바꾸기 on a connected bot.
    case "access_mode": {
      const mode = modeOf(p);
      const id = typeof p.id === "string" ? p.id : null;
      if (!id || !mode || !(id in model.modes) || model.modes[id] === mode) return null;
      return { ...model, modes: { ...model.modes, [id]: mode } };
    }
    // The world's own beat: the 전체 bot's agent asks.
    case "practice_bot_asks":
      return model.asked ? null : { ...model, asked: true };
    // Refusing or disconnecting would leave the chapter with nothing to show; the practice keeps both bots.
    default: {
      const desk = applyDesk(model.desk, event);
      return desk ? { ...model, desk } : null;
    }
  }
}

function connection(id: string, name: string, program: string, mode: BotModeKind, nowMs: number, agents: number): AccessConnection {
  return { id, name, mode: { kind: mode }, program, created_ms: nowMs - 2 * 24 * 60 * MINUTE_MS, last_used_ms: nowMs - MINUTE_MS, agent_count: agents, mission_count: 0 };
}

/** Settings › 봇 as the core sends it: the staged request until it is approved, and the connected bots in the order they were approved. */
function access(model: BotsModel, nowMs: number): AccessSection {
  const connections = [connection(FULL.connection, FULL.name, FULL.program, model.modes[FULL.connection]!, nowMs, model.asked ? 1 : 0)];
  const own = model.modes[ASKING.connection];
  if (own) connections.push(connection(ASKING.connection, ASKING.name, ASKING.program, own, nowMs, 0));
  const pending = approved(model) ? [] : [{ id: ASKING.request, name: ASKING.name, program: ASKING.program, requested_ms: nowMs - 20 * SECOND_MS }];
  return { machine: "This Mac", limit: 16, pending, connections };
}

/** The 전체 bot's agent asking, as the core sends a row an outside program's agent started for the operator. */
function botRow(model: BotsModel): AgentRow {
  const at = (model.approvedAt ?? Date.now()) + ASKS_AFTER_MS;
  return agent({
    pane_id: BOT_PANE, identity_label: TITLE.deploy, agent_kind: "claude", group: "needs_you", mark: "turn", status_code: "question", demand: "question", activity: "idle", unread: true, elapsed: "2s",
    tool: { connection: FULL.connection, name: FULL.name, owns: false },
    request: rowRequest(TITLE.deploy, TITLE.deploy, at, FULL.name),
    state: rowState("asking", true, at, { ask: { verb: "answer", what: QUESTION, more: 0 }, line: { text: QUESTION, mode: "request", tone: { kind: "warning", read: false } } }),
  });
}

/** The desk with the 전체 bot's agent in a tab of its own once it asked, behind the learner's. */
function deskOf(model: BotsModel): DeskModel {
  if (!model.asked) return model.desk;
  const desk = model.desk;
  return {
    ...desk,
    panes: [...desk.panes, { id: BOT_PANE, agent: botRow(model), title: TITLE.deploy }],
    tabs: [...desk.tabs, { id: "tbot", name: null, root: { type: "pane", pane_id: BOT_PANE }, focused: BOT_PANE, zoomed: false }],
    groups: desk.groups.map((group, index) => (index === 0 ? { ...group, tabs: [...group.tabs, "tbot"] } : group)),
  };
}

function snapshot(model: BotsModel, nowMs: number): WorldSnapshot {
  const drawn = deskSnapshot([{ place: PLACE, model: deskOf(model) }], PLACE.project, nowMs);
  const rest = model.asked ? counted(drawn.rest, "needs_you") : drawn.rest;
  const navigator = rest.navigator!;
  // A bot in its own space has a cell beside the devices; a 전체 bot's work is the operator's and has none.
  const tools = Object.entries(model.modes).filter(([, mode]) => mode === "own").map(([id]) => ({ connection: id, name: id === ASKING.connection ? ASKING.name : FULL.name, turn: 0, devices: [] }));
  const headers: Record<string, PaneHeader> = { lead: { working: true, notice: null, band: null, request: { text: TITLE.lead, images: 0 }, progress: "Editing src/payment/checkout.ts" } };
  if (model.asked) headers[BOT_PANE] = { working: false, notice: null, band: { kind: "answer", reason: QUESTION, since_unix_ms: (model.approvedAt ?? nowMs) + ASKS_AFTER_MS, read: false, action: null, more: 0 }, request: { text: TITLE.deploy, images: 0 } };
  return {
    ...drawn,
    rest: { ...rest, navigator: { ...navigator, tools }, ui_state: { ...rest.ui_state, device_rail_visible: true }, terminal: { headers } },
    frames: { access: access(model, nowMs) },
  };
}

const CLAUDE = "\x1b[38;2;215;119;87m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const RESET = "\x1b[0m";

function screen(_model: BotsModel, paneId: string, cols: number): string {
  const rule = `${DIM}${"─".repeat(cols)}${RESET}`;
  if (paneId === BOT_PANE) {
    return [
      "", `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`,
      "", `${DIM}>${RESET} ${TITLE.deploy}`, "",
      `${CLAUDE}●${RESET} The build passed and the notes are written.`, "",
      rule, ` ${BOLD}${QUESTION}${RESET}`, "", ` ${CLAUDE}› 1. Yes, push v2.4.0${RESET}`, "   2. Not yet",
    ].join("\r\n");
  }
  return [
    "", `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`,
    "", `${DIM}>${RESET} Checkout returns a 500 when I pay. Fix it`, "",
    `${CLAUDE}●${RESET} ${BOLD}Update${RESET}(src/payment/checkout.ts)`, `  ${DIM}⎿  Catch the timeout and retry once${RESET}`, "",
    `${CLAUDE}✻${RESET} ${DIM}Running the payment tests… (esc to interrupt)${RESET}`,
  ].join("\r\n");
}

export const BOTS_WORLD: PracticeWorld<BotsModel> = {
  initial,
  apply,
  snapshot,
  panes: (model) => deskOf(model).panes.map((pane) => pane.id),
  screen,
  // The 전체 bot's agent asks a few seconds after the approval, as an outside program's agent would.
  later: (model) => (model.approvedAt !== null && !model.asked ? { at: model.approvedAt + ASKS_AFTER_MS, event: { kind: "practice_bot_asks", payload: {} } } : null),
};

