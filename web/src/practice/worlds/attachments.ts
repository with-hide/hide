// Flow 12's world: one invented project with an agent that measured memory and reported, and the learner's phone beside the
// screen, open on that agent's conversation. The phone's `+` stands in for the system's picker: it attaches a screenshot and a log
// as a pick would, and leaves out a 14 MB trace with its line, since a file past 10 MB is never sent. The log uploads for a moment
// with its band; send types the files' paths and the words into the agent's own screen as one message, the files stand over the
// bubble, and the agent answers. A practice screen has no phone, no picker and no relay, so the phone is drawn beside the screen
// from the phone's own parts (`PracticePhone.tsx`). Nothing leaves the page.

import { sampleShot } from "../../components/sample-shot";
import type { ComposerFile, SentFile } from "../../mobile/uploads";
import type { CoreEvent, PracticePhoneScreen, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, deskSnapshot, oneTabDesk, rowRequest, rowState, type DeskModel, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "perf/scrollback" };
/** The pane of the agent the files go to. */
export const MEASURING = "memory";
/** How long the log takes to upload, and how long after the reply the agent answers. */
const UPLOAD_MS = 2_400;
const ANSWERS_AFTER_MS = 2_000;
const MINUTE_MS = 60_000;

const TITLE = "Memory use";
const REPORT = "Halving the buffer took busy from 248 MB to 201 MB. Panes left open a long time still grow.";
const REPLY = "Yesterday's screen and today's holder log. See what differs.";
const ANSWERS: Record<string, string> = {
  "photo,log": "Got the screenshot and the log. After 16:40 the scrollback lines triple; I'll start there.",
  photo: "Got the screenshot. The busy line climbs after 16:40; send the holder log and I'll match it.",
  log: "Got the log. After 16:40 the scrollback lines triple; I'll start there.",
};
const FILES = [
  { id: "photo", name: "IMG_2041.jpg", size: 1_258_291, kind: "photo" },
  { id: "log", name: "holder.log", size: 348_160, kind: "file" },
] as const;
/** The file a pick leaves out: past the 10 MB a file may be. */
const TRACE = { name: "trace.json", size: 14 * 1024 * 1024 };

/** What the world can change: the desk, when the files were picked and which were taken off, whether the log is in, and when the reply went and the agent answered. */
export type AttachmentsModel = { desk: DeskModel; reportedAt: number; pickedAt: number | null; removed: readonly string[]; uploaded: boolean; sentAt: number | null; answeredAt: number | null };

/** Whether files stand on the reply bar. */
export const picked = (model: AttachmentsModel) => model.pickedAt !== null;
/** Whether every file on the bar is in, so the reply may go. */
export const uploaded = (model: AttachmentsModel) => model.uploaded;
/** Whether the reply went with its files. */
export const sent = (model: AttachmentsModel) => model.sentAt !== null;
/** Whether the agent answered the files. */
export const answered = (model: AttachmentsModel) => model.answeredAt !== null;

const kept = (model: AttachmentsModel) => FILES.filter((file) => !model.removed.includes(file.id));

function initial(): AttachmentsModel {
  return { desk: oneTabDesk({ id: MEASURING, agent: null, title: TITLE }), reportedAt: Date.now() - 4 * MINUTE_MS, pickedAt: null, removed: [], uploaded: false, sentAt: null, answeredAt: null };
}

function apply(model: AttachmentsModel, event: CoreEvent): AttachmentsModel | null {
  switch (event.kind) {
    // The `+`: the picker's choice, a screenshot and a log, stands on the bar; the trace is left out.
    case "practice_phone_attach":
      return picked(model) || sent(model) ? null : { ...model, pickedAt: Date.now(), removed: [], uploaded: false };
    // A file's `×` takes only that file off.
    case "practice_phone_remove": {
      const id = typeof event.payload.id === "string" ? event.payload.id : null;
      if (!picked(model) || sent(model) || !id || !kept(model).some((file) => file.id === id)) return null;
      const removed = [...model.removed, id];
      return removed.length === FILES.length ? { ...model, pickedAt: null, removed: [], uploaded: false } : { ...model, removed };
    }
    // The world's own beat: the log's last bytes are in.
    case "practice_upload_done":
      return picked(model) && !uploaded(model) ? { ...model, uploaded: true } : null;
    // Send: the words and the files go as one message.
    case "practice_phone_send":
      return picked(model) && uploaded(model) && !sent(model) ? { ...model, sentAt: Date.now() } : null;
    // The world's own beat: the agent read the files and answers.
    case "practice_agent_answers":
      return sent(model) && !answered(model) ? { ...model, answeredAt: Date.now() } : null;
    default: {
      const desk = applyDesk(model.desk, event);
      return desk ? { ...model, desk } : null;
    }
  }
}

/** The agent's row: finished with its report until the files come, then at work on them, then finished again. */
function deskOf(model: AttachmentsModel): DeskModel {
  const working = sent(model) && !answered(model);
  const since = model.answeredAt ?? model.sentAt ?? model.reportedAt;
  const row = agent({
    pane_id: MEASURING, identity_label: TITLE, agent_kind: "claude",
    group: working ? "working" : "seen", mark: working ? "going" : "quiet", status_code: working ? "working" : "done",
    activity: working ? "working" : "stopped", elapsed: working ? "0s" : "4m",
    request: rowRequest(sent(model) ? REPLY : TITLE, TITLE, since, null),
    state: rowState(working ? "working" : "quiet", true, since),
  });
  return { ...model.desk, panes: model.desk.panes.map((pane) => (pane.id === MEASURING ? { ...pane, agent: row } : pane)) };
}

function snapshot(model: AttachmentsModel, nowMs: number): WorldSnapshot {
  return deskSnapshot([{ place: PLACE, model: deskOf(model) }], PLACE.project, nowMs);
}

/** The files as the reply sent them; the photo's thumbnail is the phone's own, kept for this conversation. */
function sentFiles(model: AttachmentsModel, thumb: string): SentFile[] {
  return kept(model).map((file) => ({ stage: file.id, name: file.name, size: file.size, kind: file.kind, thumb: file.kind === "photo" ? thumb : null }));
}

/** The phone's detail of the agent: its report, the reply bar with its files until it goes, then the files over the bubble and the answer. */
function phone(model: AttachmentsModel): PracticePhoneScreen {
  const thumb = sampleShot();
  const said: PracticePhoneScreen["said"] = [{ who: "agent", text: REPORT }];
  const attached: Record<number, SentFile[]> = {};
  if (sent(model)) {
    attached[said.length] = sentFiles(model, thumb);
    said.push({ who: "you", text: REPLY });
  }
  if (answered(model)) said.push({ who: "agent", text: ANSWERS[kept(model).map((file) => file.id).join(",")]! });
  const files: ComposerFile[] = picked(model) && !sent(model)
    ? kept(model).map((file) => {
        const up = file.kind === "photo" || uploaded(model);
        return { id: file.id, name: file.name, size: file.size, kind: file.kind, thumb: file.kind === "photo" ? thumb : null, state: up ? "ready" : "uploading", sent: up ? file.size : Math.round(file.size * 0.4), reason: null };
      })
    : [];
  return {
    pane: MEASURING, title: TITLE, kind: "claude", said, ask: null, picked: null, answered: false, running: null, attached,
    reply: {
      text: picked(model) && !sent(model) ? REPLY : "",
      files,
      notice: picked(model) && !sent(model) ? { kind: "too_large", name: TRACE.name, size: TRACE.size } : null,
      sendable: picked(model) && uploaded(model) && !sent(model),
    },
  };
}

const CLAUDE = "\x1b[38;2;215;119;87m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const RESET = "\x1b[0m";

/** The agent's own screen: its report, then the message the phone typed, the files' paths before the words, and its reading. */
function screen(model: AttachmentsModel): string {
  const head = ["", `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`, "", `${DIM}>${RESET} Measure the holder's memory with the smaller buffer`, "", `${CLAUDE}●${RESET} ${REPORT}`, ""];
  if (!sent(model)) return [...head, `${DIM}>${RESET} `].join("\r\n");
  const paths = kept(model).map((file) => `"~/.hide/state/attachments/${file.id}-${file.name}"`).join(" ");
  const reading = kept(model).map((file) => `${CLAUDE}●${RESET} ${BOLD}Read${RESET}(${file.name})`);
  const after = answered(model) ? [...reading, "", `${CLAUDE}●${RESET} ${ANSWERS[kept(model).map((file) => file.id).join(",")]}`] : [`${DIM}✻ Reading…${RESET}`];
  return [...head, `${DIM}>${RESET} ${paths} ${REPLY}`, "", ...after].join("\r\n");
}

export const ATTACHMENTS_WORLD: PracticeWorld<AttachmentsModel> = {
  initial,
  apply,
  snapshot,
  panes: (model) => deskOf(model).panes.map((pane) => pane.id),
  screen,
  phone,
  // The log is in a moment after the pick, and the agent answers a moment after the reply, as real ones do.
  later: (model) => {
    if (picked(model) && !uploaded(model)) return { at: model.pickedAt! + UPLOAD_MS, event: { kind: "practice_upload_done", payload: {} } };
    if (sent(model) && !answered(model)) return { at: model.sentAt! + ANSWERS_AFTER_MS, event: { kind: "practice_agent_answers", payload: {} } };
    return null;
  },
};
