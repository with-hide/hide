// Flow 9's world: one invented project with the learner's agent at work, and
// another computer that runs Hide, a Mac mini, which shared a link to open it.
// The other computer cannot be on a practice screen, so its side is staged:
// the link it shared is the one the step pastes, and it answers as hided does
// (`pairing_join`, `pairing_compare`, `pairing_dismiss_join`): the pairing
// window shows the four words, then that they are connected. The Mac mini is
// then a computer this one opens, as the core and hided tell it: a device with
// that machine's key, its tile on the rail, and, a few seconds later, the
// question its own agent asks, told as that computer's own work. Nothing
// leaves the page.

import type { MessageKey } from "../../i18n/catalogs";
import { clientI18n } from "../../i18n/translator";
import type { PairingResult, PairingView, Peer } from "../../pairing";
import type { Device, PeerWork } from "../../snapshot";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, deskSnapshot, oneTabDesk, type DeskModel, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };

/** The link the Mac mini shared; the step pastes it, and hided takes nothing else. */
export const LINK = "hide://pair#mac-mini.4f7c2a9e1b";
/** The other computer: its connection id, its product device id and its name. */
export const MINI = { key: "4f".repeat(32), device: "mac-mini", name: "Mac mini" } as const;
/** How long after the pairing the Mac mini's own agent asks its question. */
const ASKS_AFTER_MS = 3_000;

/**
 * What the world can change: the desk, where the pairing stands (`comparing`
 * once the link is pasted, `paired` once the words matched, `closed` once its
 * window is closed), when it was paired, and whether the Mac mini's agent has
 * asked. `request` is the Connect press hided answered, so the add window
 * hands over to the pairing window.
 */
export type ConnectModel = { desk: DeskModel; pairing: "none" | "comparing" | "paired" | "closed"; request: string | null; pairedAt: number | null; asked: boolean };

/** Whether the learner pasted the link and pressed Connect. */
export const joined = (model: ConnectModel) => model.pairing !== "none";
/** Whether the words matched and the two computers are connected. */
export const paired = (model: ConnectModel) => model.pairedAt !== null;
/** Whether the Mac mini's agent is waiting on the learner. */
export const miniAsks = (model: ConnectModel) => model.asked;

const TITLE = "Fix the checkout 500";

function initial(): ConnectModel {
  const lead = agent({ pane_id: "lead", identity_label: TITLE, agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "4m" });
  return { desk: oneTabDesk({ id: "lead", agent: lead, title: TITLE }), pairing: "none", request: null, pairedAt: null, asked: false };
}

function apply(model: ConnectModel, event: CoreEvent): ConnectModel | null {
  const p = event.payload;
  switch (event.kind) {
    // The link the Mac mini shared, pasted and sent: the two computers compare their words.
    case "pairing_join":
      return model.pairing === "none" && typeof p.link === "string" && p.link.trim() === LINK ? { ...model, pairing: "comparing", request: typeof p.request_id === "string" ? p.request_id : null } : null;
    // They match: connected. A mismatch would end the pairing, which leaves the chapter nothing to show, so the words stay.
    case "pairing_compare":
      return model.pairing === "comparing" && p.same === true ? { ...model, pairing: "paired", pairedAt: Date.now() } : null;
    case "pairing_dismiss_join":
      return model.pairing === "paired" ? { ...model, pairing: "closed" } : null;
    // The world's own beat: the Mac mini's agent asks.
    case "practice_mini_asks":
      return model.asked ? null : { ...model, asked: true };
    default: {
      const desk = applyDesk(model.desk, event);
      return desk ? { ...model, desk } : null;
    }
  }
}

/** This chapter's words in the learner's language (`practice.connect.*`), added to the catalog when the chapters load. */
const say = (key: string) => clientI18n.t(`practice.connect.${key}` as MessageKey) as string;

/** The four words both screens show, from the list hided reads for the learner's language (`hide-peer` `words`). */
const words = () => say("words").split(" ");

/** hided's `pairing` frame: the pairing this computer is joining, until its window is closed. */
function pairingFrame(model: ConnectModel, nowMs: number): PairingView {
  const join = model.pairing === "comparing" || model.pairing === "paired"
    ? { id: 1, host: MINI.key, phase: model.pairing, failure: null, name: MINI.name, words: words(), expires_unix_ms: nowMs + 9 * 60_000 }
    : null;
  return { offer: null, asks: [], join, requests: [] };
}

const PEER: Peer = { key: MINI.key, device_id: MINI.device, name: MINI.name, opens: "active", opened_by: null, connected: true, path: "direct", reason: null, update: null };

/** What the Mac mini told of its own work: nothing yet, then its agent's question. */
function work(model: ConnectModel, nowMs: number): PeerWork {
  return {
    device_id: MINI.device, key: MINI.key, needs_you: model.asked ? 1 : 0, done: 0,
    rows: model.asked
      ? [{ id: "w1:p1", group: "needs_you", title: say("miniTask"), agent_kind: "codex", place: "nightly-build · main", mark: "turn", status_code: "question", detail: say("miniQuestion"), read: false, changed_at_unix_ms: nowMs }]
      : [],
  };
}

function snapshot(model: ConnectModel, nowMs: number): WorldSnapshot {
  const drawn = deskSnapshot([{ place: PLACE, model: model.desk }], PLACE.project, nowMs);
  const navigator = drawn.rest.navigator!;
  const local = navigator.devices![0]!;
  // The core makes a device of a machine this one opens, under that machine's key (`peer_devices`).
  const mini: Device = { ...local, id: MINI.device, label: MINI.name, kind: "remote", state: "ready", ssh_alias: null, agent_count: 0, peer_key: MINI.key };
  const connected = paired(model);
  return {
    ...drawn,
    rest: {
      ...drawn.rest,
      navigator: {
        ...navigator,
        devices: connected ? [local, mini] : [local],
        peers: connected ? [PEER] : [],
        peer_work: connected ? [work(model, nowMs)] : [],
      },
      status: { ...drawn.rest.status, remote: connected ? [{ target_id: MINI.device, state: "connected", message: null }] : [] },
      ui_state: { ...drawn.rest.ui_state, device_rail_visible: true },
    },
    frames: { pairing: pairingFrame(model, nowMs), pairingResult: model.request ? ({ verb: "join", ok: true, request_id: model.request } satisfies PairingResult) : null },
  };
}

const CLAUDE = "\x1b[38;2;215;119;87m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const RESET = "\x1b[0m";

function screen(): string {
  return [
    "", `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`,
    "", `${DIM}>${RESET} Checkout returns a 500 when I pay. Fix it`, "",
    `${CLAUDE}●${RESET} ${BOLD}Update${RESET}(src/payment/checkout.ts)`, `  ${DIM}⎿  Catch the timeout and retry once${RESET}`, "",
    `${CLAUDE}✻${RESET} ${DIM}Running the payment tests… (esc to interrupt)${RESET}`,
  ].join("\r\n");
}

export const CONNECT_WORLD: PracticeWorld<ConnectModel> = {
  initial,
  apply,
  snapshot,
  panes: (model) => model.desk.panes.map((pane) => pane.id),
  screen,
  // The Mac mini's agent asks a few seconds after the two connect, as work there would.
  later: (model) => (model.pairedAt !== null && !model.asked ? { at: model.pairedAt + ASKS_AFTER_MS, event: { kind: "practice_mini_asks", payload: {} } } : null),
};
