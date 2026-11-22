// Flow 8's world: one invented project with the learner's agent at work and a
// bot already connected as 전체. The learner makes a second bot in Settings ›
// 봇, and the world answers as the core does (`access_call`): the bot waits for
// its AI with an invented code, and the learner copies the sentence for that AI.
// An outside AI running `hide access join` is something a practice screen
// cannot run, so the join is staged: a few seconds after the learner sent the
// sentence, its AI joins, the bot is connected in the mode picked and, in its
// own space, its owl lands on the rail. A few seconds later the 전체
// bot's agent asks the operator a question, and its row stands in 내 차례
// wearing that bot's owl. Nothing leaves the page.

import type { AccessCall } from "../../accessCall";
import type { BotModeKind } from "../../bots";
import { BOT_NAME } from "../../externalTools";
import type { AccessConnection, AccessInvite, AccessSection, AgentRow, PaneHeader } from "../../snapshot";
import { PRACTICE_COPY, type CoreEvent, type PracticeWorld, type WorldSnapshot } from "../world";
import { agent, applyDesk, counted, deskSnapshot, oneTabDesk, rowRequest, rowState, type DeskModel, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };
const SECOND_MS = 1_000;
const MINUTE_MS = 60 * SECOND_MS;

/** The bot the learner makes, by the id the core would draw, and the 전체 bot already connected. */
export const MADE = { id: "made-bot", program: "/usr/local/bin/hermes-agent" } as const;
export const FULL = { connection: "conn-hermes", name: "hermes", program: "/usr/local/bin/hermes" } as const;
/** The code the made bot's sentence carries: invented, and nothing joins with it. */
const CODE = "K7Q4-MZ2P-X3RD";
const CODE_MS = 10 * MINUTE_MS;
/** The pane of the agent the 전체 bot started, which asks once the made bot's AI joined. */
export const BOT_PANE = "deploy";
/** How long after the sentence went its AI joins, and after that the 전체 bot's agent asks. */
const JOINS_AFTER_MS = 4_000;
const ASKS_AFTER_MS = 2_500;

/** Told to the world as the learner starts waiting for the made bot's AI: the sentence went, and the AI joins a few seconds later. */
export const SENT = "practice_bot_sent";

/**
 * What the world can change: the desk, the mode each connected bot works in,
 * the bot the learner made and when (it waits until its AI joins), whether
 * the learner copied its sentence, when the sentence went, when its AI
 * joined, and whether the 전체 bot's agent has asked.
 */
export type BotsModel = {
  desk: DeskModel;
  modes: Record<string, BotModeKind>;
  made: { name: string; mode: BotModeKind; at: number } | null;
  copied: boolean;
  sentAt: number | null;
  joinedAt: number | null;
  asked: boolean;
};

/** Whether the learner made a bot. */
export const made = (model: BotsModel) => model.made !== null;
/** Whether the learner copied the made bot's sentence. */
export const copied = (model: BotsModel) => model.copied;
/** Whether the made bot's AI joined. */
export const joined = (model: BotsModel) => model.joinedAt !== null;
/** Whether the 전체 bot's agent is waiting on the learner in 내 차례. */
export const botAsks = (model: BotsModel) => model.asked;

const TITLE = { lead: "Fix the checkout 500", deploy: "Deploy the release" };
const QUESTION = "Push the tag now?";

function initial(): BotsModel {
  const lead = agent({ pane_id: "lead", identity_label: TITLE.lead, agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "4m" });
  return { desk: oneTabDesk({ id: "lead", agent: lead, title: TITLE.lead }), modes: { [FULL.connection]: "all" }, made: null, copied: false, sentAt: null, joinedAt: null, asked: false };
}

const modeOf = (payload: Record<string, unknown>): BotModeKind | null => {
  const kind = (payload.mode as { kind?: unknown } | undefined)?.kind;
  return kind === "own" || kind === "all" ? kind : null;
};

function apply(model: BotsModel, event: CoreEvent): BotsModel | null {
  const p = event.payload;
  switch (event.kind) {
    // The sentence on the practice clipboard: what the learner would send to the AI.
    case PRACTICE_COPY:
      return p.what === "bot sentence" && model.made && !model.copied ? { ...model, copied: true } : null;
    // The learner waits for the AI: it joins a few seconds from now.
    case SENT:
      return model.made && !joined(model) ? { ...model, sentAt: Date.now() } : null;
    // The made bot's AI ran `hide access join`: the bot is connected in its mode from then on.
    case "practice_bot_joins": {
      if (!model.made || joined(model)) return null;
      return { ...model, modes: { ...model.modes, [MADE.id]: model.made.mode }, joinedAt: Date.now() };
    }
    // Taking the made bot back before its AI joined: the learner can make another.
    case "access_invite_cancel":
      return p.id === MADE.id && model.made && !joined(model) ? { ...model, made: null, copied: false, sentAt: null } : null;
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

/** The made bot as Settings' call answers it, with the invented code. */
function madeAnswer(made: NonNullable<BotsModel["made"]>) {
  return { id: MADE.id, name: made.name, mode: { kind: made.mode }, code: CODE, expires_ms: made.at + CODE_MS };
}

/** Settings' call, answered as the core would: a bot made once, by a name no bot has, and its code read again while it waits. */
function call(model: BotsModel, asked: AccessCall): { model: BotsModel | null; answer: { ok: true; value: unknown } | { ok: false; code: string } } {
  if (asked.call === "invite") {
    if (!BOT_NAME.test(asked.name)) return { model: null, answer: { ok: false, code: "invalid_request" } };
    if (model.made || asked.name.toLowerCase() === FULL.name) return { model: null, answer: { ok: false, code: "access_name_taken" } };
    const next = { ...model, made: { name: asked.name, mode: asked.mode.kind, at: Date.now() } };
    return { model: next, answer: { ok: true, value: madeAnswer(next.made) } };
  }
  if (!model.made || joined(model) || asked.id !== MADE.id) return { model: null, answer: { ok: false, code: "access_invite_gone" } };
  if (asked.call === "renew") {
    const next = { ...model, made: { ...model.made, at: Date.now() } };
    return { model: next, answer: { ok: true, value: madeAnswer(next.made) } };
  }
  return { model: null, answer: { ok: true, value: madeAnswer(model.made) } };
}

function connection(id: string, name: string, program: string, mode: BotModeKind, nowMs: number, agents: number): AccessConnection {
  return { id, name, mode: { kind: mode }, program, created_ms: nowMs - 2 * 24 * 60 * MINUTE_MS, last_used_ms: nowMs - MINUTE_MS, agent_count: agents, mission_count: 0 };
}

/** Settings › 봇 as the core sends it: the made bot while it waits for its AI, and the connected bots in the order they connected. */
function access(model: BotsModel, nowMs: number): AccessSection {
  const connections = [connection(FULL.connection, FULL.name, FULL.program, model.modes[FULL.connection]!, nowMs, model.asked ? 1 : 0)];
  const mode = model.modes[MADE.id];
  if (model.made && mode) connections.push(connection(MADE.id, model.made.name, MADE.program, mode, nowMs, 0));
  const invites: AccessInvite[] = model.made && !joined(model) ? [{ id: MADE.id, name: model.made.name, mode: { kind: model.made.mode }, created_ms: model.made.at, expires_ms: model.made.at + CODE_MS }] : [];
  return { machine: "This Mac", limit: 16, pending: [], invites, connections };
}

/** The 전체 bot's agent asking, as the core sends a row an outside program's agent started for the operator. */
function botRow(model: BotsModel): AgentRow {
  const at = (model.joinedAt ?? Date.now()) + ASKS_AFTER_MS;
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
  const tools = Object.entries(model.modes).filter(([, mode]) => mode === "own").map(([id]) => ({ connection: id, name: id === MADE.id ? model.made?.name ?? id : FULL.name, turn: 0, devices: [] }));
  const headers: Record<string, PaneHeader> = { lead: { working: true, notice: null, band: null, request: { text: TITLE.lead, images: 0 }, progress: "Editing src/payment/checkout.ts" } };
  if (model.asked) headers[BOT_PANE] = { working: false, notice: null, band: { kind: "answer", reason: QUESTION, since_unix_ms: (model.joinedAt ?? nowMs) + ASKS_AFTER_MS, read: false, action: null, more: 0 }, request: { text: TITLE.deploy, images: 0 } };
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
  access: call,
  // The made bot's AI joins a few seconds after the sentence went, and the 전체 bot's agent asks a few seconds after that, as outside programs would.
  later: (model) => {
    if (model.made && model.sentAt !== null && model.joinedAt === null) return { at: model.sentAt + JOINS_AFTER_MS, event: { kind: "practice_bot_joins", payload: {} } };
    if (model.joinedAt !== null && !model.asked) return { at: model.joinedAt + ASKS_AFTER_MS, event: { kind: "practice_bot_asks", payload: {} } };
    return null;
  },
};

