// Desk's screen values (PRD session-docs-and-assistant, Desk 1차): what the
// Desk room, its job cards, the Threads list and a thread draw. The wire's
// own shapes are `snapshot.ts`'s (the `desk`, `desk_live`, `desk_jobs` and
// `desk_mentions` sections), re-exported here under the names the parts use;
// the types below them are the screen's own. The parts take these values as
// props only; nothing here reads the store or the socket.
//
// The fields no section carries (marked `wire gap` below) are a thread's
// own, which the threads node reads from the session record.

import { sentenceIn } from "./markdownPlain";
import type { AgentRow, DeskAgent, DeskAgentState, DeskAskKind, DeskItem, DeskOption, DeskOptionKind, DeskJob, DeskJobsSection, DeskMoved, DeskRecord, DeskMention as WireMention, DeskMission, DeskMissionMention, DeskNotice, DeskOutput, DeskOutputKind, DeskSection, DeskSessionChip, DeskStepKind, PlanView, PrState, RowMark } from "./snapshot";

export type { DeskAgent, DeskAgentId, DeskRecord, DeskAgentState, DeskApprovalState, DeskChoice, DeskItem, DeskJob, DeskJobFailure, DeskJobState, DeskLive, DeskLocation, DeskMission, DeskMissionMention, DeskOption, DeskOptionKind, DeskOutput, DeskOutputKind, DeskPermission, DeskQueuedMessage, DeskSteps } from "./snapshot";

export type DeskSelection = DeskSection["selection"];

/** Whether Desk is switched on in Settings › Labs › Desk (D-76, B41); before the section arrives it is not. */
export function deskOn(desk: DeskSection | null | undefined): boolean {
  return desk?.settings.enabled === true;
}
export type DeskSettings = DeskSection["settings"];
export type DeskNoticeKind = DeskNotice;
/** What an asking job waits on, each said with its own verb (B20): a question, a terminal's permission, a mission's decision, a plan to review, a call that stopped. */
export type DeskAsk = DeskAskKind;

/** A mention as a chip draws it: the wire's, and for a session its status and agent, so its chip reads as a session does everywhere. */
export type DeskMention = WireMention & { mark?: RowMark | null; agent_kind?: string | null };

/** A session a reply names, as its chip draws it: the token is its pane, as the reply's text spells it. */
export function chipMention(chip: DeskSessionChip): DeskMention {
  return { kind: "session", id: chip.pane_id, name: chip.name, mark: chip.mark, agent_kind: chip.agent_kind };
}

/**
 * A session as a chip names it: its status, its agent and its name (B26); a
 * session Hide does not know the state of has its name alone. `pane_id` is
 * where it runs, the place pressing its chip goes, when Hide knows it.
 */
export type DeskSessionRef = { name: string; mark: RowMark | null; agent_kind: string | null; pane_id?: string | null };

/** The session a handed line names, from the line's own mark and agent. */
export function handedSession(item: Extract<DeskItem, { kind: "handed" }>): DeskSessionRef {
  return { name: item.name, mark: item.mark, agent_kind: item.agent_kind, pane_id: item.pane_id };
}

/** Every card Desk has (D-86): its jobs, their missions' workers, and the other sessions its conversation names or opened beside it. */
export function laneCards(section: DeskJobsSection | null | undefined): DeskJob[] {
  if (!section) return [];
  return [...section.jobs, ...section.workers, ...section.named];
}

/** The card of the session a chip names: by its pane when the chip knows it, else by its name. */
export function cardOfSession(cards: readonly DeskJob[], session: DeskSessionRef): DeskJob | null {
  return cards.find((card) => (session.pane_id ? card.pane_id === session.pane_id : card.title === session.name)) ?? null;
}

/**
 * What Hide knows of a session a chip names, from its sidebar row and its
 * card (D-86): whether the sidebar still lists it, its row's name, mark and
 * agent, its card (the facts its hover card shows and the thread a press
 * opens), the time its row shows and its parent.
 */
export type SessionFacts = {
  listed: boolean;
  /** Where it runs now, the pane a press opens: its row's (a gone pane's session resumed in another pane is that pane's, `desk_jobs.moved`), the chip's own for a recorded session, null for one with nothing to open. */
  pane_id: string | null;
  /** The name its sidebar row shows, which a chip draws in place of the name the words wrote (an agent's reply may name it by an id). */
  name: string | null;
  mark: RowMark | null;
  agent_kind: string | null;
  card: DeskJob | null;
  /** When its row's time counts from (`last_prompt_at_unix_ms`). */
  since: number | null;
  parent: DeskSessionRef | null;
  /** What Hide's record keeps of a session the sidebar no longer lists (frame K): its chip draws it, a press opens it beside Desk read-only; null for a listed session or one with no record. */
  record: DeskRecord | null;
};

/** A row as a chip names it. */
function rowSession(row: AgentRow): DeskSessionRef {
  return { name: row.identity_label, mark: row.mark, agent_kind: row.agent_kind, pane_id: row.pane_id };
}

/**
 * The job states that end a job for Desk: no pane is coming for it, until a
 * failed or cancelled one is started again, or its tab left Desk. Desk's room
 * passes over such a job for the turn's next session, and its card is dimmed
 * while its agent is not at work or asking (`jobDimmed`).
 */
export const ENDED_JOB_STATES: ReadonlySet<DeskJob["state"]> = new Set<DeskJob["state"]>(["failed", "cancelled", "closed", "moved_out", "archived"]);

/**
 * Whether a job's agent is at work as Desk counts it: its row says it works,
 * whatever the job's place (moved out of Desk's group included). A job with
 * no row read says nothing of its agent; the place it holds is the core's.
 */
export function jobWorking(job: DeskJob): boolean {
  return job.activity === "working";
}

/** Where a session stands in 한눈에: waiting on the operator, at work, or idle (its agent done or asleep, or its job ended with nothing at work), shown as 끝남. */
export type GlanceGroup = "turn" | "working" | "idle";

export function glanceGroup(job: DeskJob): GlanceGroup {
  if (job.activity === "asking") return "turn";
  if (job.state === "queued" || job.state === "starting" || jobWorking(job)) return "working";
  return "idle";
}

const GLANCE_ORDER: Readonly<Record<GlanceGroup, number>> = { turn: 0, working: 1, idle: 2 };

/**
 * 한눈에's sessions as cards (`desk_jobs.glance.sessions`, newest first): 내
 * 차례 first, then at work, then ended, each newest first; a pane with no
 * card is left out.
 */
export function glanceSessions(panes: readonly string[], cards: readonly DeskJob[]): DeskJob[] {
  const byPane = new Map(cards.filter((card) => card.pane_id !== null).map((card) => [card.pane_id!, card]));
  const listed = panes.flatMap((pane) => {
    const card = byPane.get(pane);
    return card ? [card] : [];
  });
  return listed.map((card, at) => ({ card, at })).sort((a, b) => GLANCE_ORDER[glanceGroup(a.card)] - GLANCE_ORDER[glanceGroup(b.card)] || a.at - b.at).map(({ card }) => card);
}

/** Whether a job card is dimmed: an ended job's, but a failed one's (see `ENDED_JOB_STATES`) and one whose agent still works or asks. */
export function jobDimmed(job: DeskJob): boolean {
  return ENDED_JOB_STATES.has(job.state) && job.state !== "failed" && job.activity !== "working" && job.activity !== "asking";
}

/**
 * The pane a chip's session runs in now: a gone pane whose session the core
 * found running in another pane is that pane (`desk_jobs.moved`), any other
 * pane itself.
 */
export function livePane(moved: readonly DeskMoved[] | undefined, pane: string): string {
  return moved?.find((one) => one.pane_id === pane)?.live_pane_id ?? pane;
}

/** What Desk's section says of the sessions whose pane is gone: their records and the ones running in another pane. */
export type GoneSessions = Pick<DeskJobsSection, "records" | "moved">;

/**
 * A chip's session as the sidebar and Desk's cards know it now (D-86): a
 * session named by its pane that no row has any more, and that runs in no
 * other pane, is not listed, so its chip draws its record or is dimmed; one
 * named only by a name no row carries is not known, and its chip keeps the
 * values it came with.
 */
export function sessionFacts(rows: ReadonlyMap<string, AgentRow>, cards: readonly DeskJob[], session: DeskSessionRef, gone: GoneSessions | null = null): SessionFacts | null {
  const row = session.pane_id ? rows.get(livePane(gone?.moved, session.pane_id)) : [...rows.values()].find((one) => one.identity_label === session.name);
  if (!row) {
    if (!session.pane_id) return null;
    const record = gone?.records.find((one) => one.pane_id === session.pane_id) ?? null;
    return { listed: false, pane_id: record ? session.pane_id : null, name: record?.title ?? null, mark: null, agent_kind: record?.agent_kind ?? null, card: record ? recordCard(record) : null, since: record?.ended_at_unix_ms ?? null, parent: null, record };
  }
  const parent = row.lineage_parent_pane_id ? rows.get(row.lineage_parent_pane_id) : undefined;
  return {
    listed: true,
    pane_id: row.pane_id,
    name: row.identity_label,
    mark: row.mark,
    agent_kind: row.agent_kind,
    card: cardOfSession(cards, { ...session, pane_id: row.pane_id }),
    since: row.last_prompt_at_unix_ms ?? null,
    parent: parent ? rowSession(parent) : null,
    record: null,
  };
}

/** A wire path's last name (`/` between names on every system), as a card's ⌂ names a folder. */
function lastName(path: string): string {
  return path.replace(/\/+$/u, "").split("/").pop() || path;
}

/**
 * A record as the app's `SessionCard` draws a session (frame K, 나): ended
 * (`끝남` with the time since it ended), ⌂ its project and the folder it
 * worked in, its last result line, and its pull requests. Every fact is the
 * record's; one it does not keep is left out.
 */
export function recordCard(record: DeskRecord): DeskJob {
  const project = record.project ?? record.cwd;
  const folder = record.cwd ? lastName(record.cwd) : "";
  return {
    id: `record:${record.session.device_id}:${record.session.agent}:${record.session.session_id}`,
    agent_id: null,
    pane_id: record.pane_id,
    title: record.title,
    agent_kind: record.agent_kind,
    project: { workspace_id: null, label: project ? lastName(project) : "" },
    location: { kind: "checkout", label: folder },
    state: "closed",
    activity: "done",
    queue_position: null,
    now: record.line,
    plan: null,
    question: null,
    summary: record.line,
    pull_requests: record.pull_requests,
    outputs: [],
    joined: false,
    reason: null,
    failure: null,
    ask: null,
    mark: null,
    mark_read: false,
    machine: null,
    disconnected: false,
    changed_at_unix_ms: record.ended_at_unix_ms ?? 0,
    read: true,
    mission: null,
    workers: [],
  };
}

export type DeskApproval = Extract<DeskItem, { kind: "approval" }>;

/** The names a JSON-RPC peer gives its own errors; an agent's refusal arrives behind one, which says nothing to the operator. */
const WIRE_ERROR_NAME = /^(?:parse error|invalid request|method not found|invalid params|internal error)\s*:\s*/i;

/**
 * The agent's reason a turn failed, in plain words (B5): without the protocol's error name, and none at all when the
 * reply above the notice already says it, since a reason is shown once. `reply` is the reply the failed turn left.
 */
export function failureReason(detail: string | null, reply: string | null): string | null {
  const reason = detail?.replace(WIRE_ERROR_NAME, "").trim() ?? "";
  if (reason === "" || reply?.includes(reason)) return null;
  return reason;
}

/**
 * The longest line `거절…` takes, in UTF-16 units: three bytes each at most
 * outside the astral planes, so it stays under the core's 2048-byte refusal
 * cap (`MAX_REFUSAL_BYTES`) and the core never drops a line typed whole.
 */
export const DESK_REFUSAL_MAX = 600;

/** The word Hide puts on one kind of the agent's options; `other` has none and shows the agent's own name. */
export type ApprovalWord = "reject" | "allow_always" | "allow_once";

/**
 * One button of a waiting card: the agent's option, and Hide's word for it,
 * or null where the agent's own name labels it. A null option is Hide's own
 * `거절…` on a card that offers no rejecting option, which ends the request
 * as ACP's `cancelled`, so no card waits on `중지`.
 */
export type ApprovalButton = { option: DeskOption | null; word: ApprovalWord | null };

const APPROVAL_WORD: Record<DeskOptionKind, ApprovalWord | null> = {
  reject_once: "reject",
  reject_always: "reject",
  allow_always: "allow_always",
  allow_once: "allow_once",
  other: null,
};

/** Where each word stands on the card, left to right: `거절…`, the agent's own others, `항상 허용`, `한 번 허용` last (D-92, frame A2 가). */
const BUTTON_ORDER: Record<ApprovalWord | "other", number> = { reject: 0, other: 1, allow_always: 2, allow_once: 3 };

/**
 * A waiting card's buttons (D-92): every option the agent offers, none left
 * out, ordered by `BUTTON_ORDER` and in the agent's order within a word. A
 * button reads Hide's word for its kind unless another option reads the same
 * word, as Claude's several `항상 허용` ways out of plan mode do; then each of
 * those reads the agent's own name, so they can be told apart.
 */
export function approvalButtons(options: readonly DeskOption[]): ApprovalButton[] {
  const words = options.map((option) => APPROVAL_WORD[option.kind]);
  const buttons: ApprovalButton[] = options
    .map((option, index) => {
      const word = words[index] ?? null;
      const shared = word !== null && words.filter((other) => other === word).length > 1;
      return { option, word: shared ? null : word, at: BUTTON_ORDER[word ?? "other"], index };
    })
    .sort((left, right) => left.at - right.at || left.index - right.index)
    .map(({ option, word }) => ({ option, word }));
  return options.some(rejects) ? buttons : [{ option: null, word: "reject" }, ...buttons];
}

/** Whether pressing the option refuses the request, which opens `거절…`'s one line first. */
export const rejects = (option: DeskOption) => option.kind === "reject_once" || option.kind === "reject_always";

/** Whether the button refuses the request: a rejecting option, or Hide's own `거절…`. */
export const refuses = ({ option }: ApprovalButton) => option === null || rejects(option);

/** One View of a session, before Desk keeps only its outputs (B30). */
export type SessionViewKind = "image" | "page" | "pr" | "markdown" | "pdf" | "html" | "code" | "diff" | "log" | "hide_ai";

export type DeskPullRequest = { number: number; state: PrState };

// ---------- the thread (B25-B27) ----------

// A thread is the session read by `hide-session::thread` (PR 1902): ACP
// `session/update` values (`agent_client_protocol_schema::v1::SessionUpdate`)
// in record order, as they serialize: `sessionUpdate` names the variant and
// fields are camelCase. Only the variants and fields the thread draws are
// typed here. wire gap: the desk draft carries no thread; the threads node
// sends these pages.

/** ACP `ContentBlock`. */
export type AcpContentBlock =
  | { type: "text"; text: string; _meta?: AcpMeta }
  | { type: "image"; data: string; mimeType: string; uri?: string | null; _meta?: AcpMeta }
  | { type: "resource_link"; uri: string; name: string };

/**
 * Hide's own marks under `_meta.hide`: a message's origin, a letter's sender as written, its kind (one of
 * `LETTER_KINDS`, from its envelope or the doorbell's line) and, for a doorbell's line, the letter's first line it
 * names; a cut text or image; when a call or its result was written (Unix milliseconds); what an agent's own question
 * call asks (`question`) and, on its result, the operator's answer to each question in order (`answers`).
 */
export type AcpMeta = {
  hide?: {
    origin?: "letter" | "interrupted";
    from?: string;
    /** The id of a letter delivered in its envelope, which a reply in the record names. */
    letter?: string;
    kind?: string;
    summary?: string;
    truncated?: boolean;
    at_ms?: number;
    question?: { text: string; choices: string[] }[];
    answers?: (string | null)[];
  };
} | null;

/** A letter's kind, as `hide letter send --kind` types it and its envelope or doorbell line names it. */
export const LETTER_KINDS = ["request", "reply", "report", "progress", "block", "watch"] as const;
export type LetterKind = (typeof LETTER_KINDS)[number];

/** A kind as a record or a command wrote it, or null for one this build does not know. */
export function letterKindOf(value: unknown): LetterKind | null {
  return (LETTER_KINDS as readonly unknown[]).includes(value) ? (value as LetterKind) : null;
}

/** ACP `ToolCallContent`. */
export type AcpToolCallContent =
  | { type: "content"; content: AcpContentBlock }
  | { type: "diff"; path: string; oldText?: string | null; newText: string; _meta?: AcpMeta }
  | { type: "terminal"; terminalId: string };

/** ACP `ToolCallStatus`. */
export type AcpToolCallStatus = "pending" | "in_progress" | "completed" | "failed";

/** ACP `ToolKind`. */
export type AcpToolKind = "read" | "edit" | "delete" | "move" | "search" | "execute" | "think" | "fetch" | "switch_mode" | "other";

/** ACP `ToolCall` fields; `ToolCallUpdate` carries any of them besides the id. */
export type AcpToolCallFields = {
  title?: string;
  kind?: AcpToolKind;
  status?: AcpToolCallStatus;
  content?: AcpToolCallContent[];
  locations?: { path: string; line?: number | null }[];
  rawInput?: unknown;
  rawOutput?: unknown;
  _meta?: AcpMeta;
};

/** ACP `PlanEntry`. */
export type AcpPlanEntry = { content: string; priority: "high" | "medium" | "low"; status: "pending" | "in_progress" | "completed" };

/** ACP `SessionUpdate`, the variants a thread holds. */
export type AcpSessionUpdate =
  | { sessionUpdate: "user_message_chunk"; content: AcpContentBlock; messageId?: string | null; _meta?: AcpMeta }
  | { sessionUpdate: "agent_message_chunk"; content: AcpContentBlock; messageId?: string | null; _meta?: AcpMeta }
  | { sessionUpdate: "agent_thought_chunk"; content: AcpContentBlock; messageId?: string | null; _meta?: AcpMeta }
  | ({ sessionUpdate: "tool_call"; toolCallId: string; title: string } & AcpToolCallFields)
  | ({ sessionUpdate: "tool_call_update"; toolCallId: string } & AcpToolCallFields)
  | { sessionUpdate: "plan"; entries: AcpPlanEntry[] };

/**
 * A `hide` call read as the Hide part it stands for (B26), keyed by the tool
 * call it replaces: a plan's steps, a new session's row, a letter's line, an
 * opened output, a mission's chip. wire gap: who reads a call into a part
 * (the threads node, from the call's `rawInput` command) and its fields.
 */
export type ThreadHidePart = (
  | { kind: "plan"; plan: PlanView }
  /** A new session this one handed work to. */
  | { kind: "spawn"; session: DeskSessionRef }
  /** A letter this session sent, to the other session, with its whole body and its kind. */
  | { kind: "letter"; peer: DeskSessionRef | null; body: string; letterKind: LetterKind | null }
  | { kind: "view"; output: DeskOutput; view_id: string | null }
  | { kind: "mission"; mission: DeskMission }
  | { kind: "pr"; number: number; state: PrState }
) & {
  /**
   * Where the part leads, by what names it in Hide rather than how it is
   * drawn: a session by its pane, else the agent id the record wrote; a View
   * by its id, else the target typed; a mission by its id. A new session, a
   * View or a mission one turn reaches again is one part
   * (`conversationTurns`); every letter is its own.
   */
  target: string;
};

/** One drawn line of a thread, after chunks are joined and call updates folded into their call. */
export type ThreadLine =
  /**
   * `peer` is a letter's sender, when the record names it; `letterKind` its kind, and `summary` the first line a
   * doorbell's line names (the line itself is `text`).
   */
  | { kind: "user"; id: string; message: string | null; text: string; images: AcpContentBlock[]; origin: "operator" | "letter" | "interrupted"; peer: DeskSessionRef | null; letterKind: LetterKind | null; summary: string | null }
  /** `atMs` is when its record was written, when the record says (its first piece's time). */
  | { kind: "agent"; id: string; message: string | null; text: string; atMs: number | null }
  | { kind: "thought"; id: string; message: string | null; text: string }
  /**
   * `startedAtMs` is when the session made the call, when its record says;
   * `subject` is what it worked on, read from its input or its first place
   * (a search's term, a file's path), whatever the agent's title says.
   */
  | { kind: "tool"; id: string; title: string; toolKind: AcpToolKind; status: AcpToolCallStatus; input: string | null; subject: string | null; output: string | null; diffs: Extract<AcpToolCallContent, { type: "diff" }>[]; images: AcpContentBlock[]; startedAtMs: number | null }
  | { kind: "plan"; id: string; entries: AcpPlanEntry[] }
  /**
   * The agent's own question call (Claude Code's `AskUserQuestion`, Codex's `request_user_input`, OpenCode's
   * `question`, Antigravity's `ask_question`, any call its reader marked `_meta.hide.question`): each question with its
   * choices and, once the operator answered, the answer; `status` is the call's.
   */
  | { kind: "question"; id: string; questions: ThreadQuestion[]; status: AcpToolCallStatus }
  /** `command` is what the session typed, when the call carries it. */
  | { kind: "hide"; id: string; part: ThreadHidePart; command: string | null };

/** One question of a question call, as the question line draws it. */
export type ThreadQuestion = { text: string; choices: string[]; answer: string | null };

export type ThreadReadFailure = "no_record" | "refused";

/**
 * A shell call's command, the one input a thread shows: any other call's
 * input (an edit's whole file, a search's options) is already said by its
 * title, its diff or its output.
 */
function commandOf(raw: unknown): string | null {
  if (typeof raw === "string") return raw;
  if (typeof raw === "object" && raw !== null && "command" in raw && typeof (raw as { command: unknown }).command === "string") return (raw as { command: string }).command;
  return null;
}

/** The input words that name what a call works on, in the order a reader trusts them: a term before a place. */
const SUBJECT_KEYS = ["pattern", "query", "q", "regex", "glob", "url", "path", "file_path", "filepath", "searchpath", "directorypath", "searchdirectory", "directory"];

/**
 * What a call works on, from the input its reader kept (a search's term, a
 * file's or a page's address), else its first place: every agent's reader
 * hands one or the other, some only the tool's name as its title.
 */
function subjectOf(raw: unknown, locations: AcpToolCallFields["locations"]): string | null {
  if (typeof raw === "object" && raw !== null) {
    const fields = new Map(Object.entries(raw).map(([key, value]) => [key.toLowerCase(), value]));
    for (const key of SUBJECT_KEYS) {
      const value = fields.get(key);
      if (typeof value === "string" && value.trim()) return value.trim();
    }
  }
  return locations?.[0]?.path ?? null;
}

/**
 * A call in one line: a shell call as its command, any other as its title,
 * followed by what it worked on when the title does not already say it
 * (`grep needle` where the reader titled it `grep`).
 */
export function callName(line: Extract<ThreadLine, { kind: "tool" }>): string {
  if (line.toolKind === "execute" && line.input) return line.input;
  return line.subject && !line.title.includes(line.subject) ? `${line.title} ${line.subject}`.trim() : line.title;
}

function textOf(block: AcpContentBlock): string {
  return block.type === "text" ? block.text : "";
}

/** A session Hide cannot place: its name alone. */
const nameOnly = (name: string): DeskSessionRef => ({ name, mark: null, agent_kind: null });

/**
 * The lines a thread draws (B25): consecutive chunks of one kind and one
 * message id joined (chunks with no message id join each other), a call's
 * updates folded into the call (an update's content replaces the call's, as
 * ACP says), and a call the reader turned into a Hide part drawn as that part
 * instead (B26). `session` names a letter's sender as its chip draws it, and
 * `ids` each update by a name a page read in above does not move
 * (`threadConversation`), so a message line keeps its id as the record grows.
 */
export function threadLines(updates: readonly AcpSessionUpdate[], parts: Readonly<Record<string, ThreadHidePart>> = {}, session: (name: string) => DeskSessionRef = nameOnly, ids: readonly string[] = []): ThreadLine[] {
  const lines: ThreadLine[] = [];
  const calls = new Map<string, Extract<ThreadLine, { kind: "tool" }>>();
  const questions = new Map<string, Extract<ThreadLine, { kind: "question" }>>();
  const answer = (line: Extract<ThreadLine, { kind: "question" }>, fields: AcpToolCallFields) => {
    if (fields.status !== undefined) line.status = fields.status;
    const answers = fields._meta?.hide?.answers;
    if (answers) line.questions = line.questions.map((question, at) => ({ ...question, answer: answers[at] ?? question.answer }));
  };
  // A letter's sender as the record wrote it, so its chunks join by who wrote them, not by how the chip names them.
  const senders = new Map<ThreadLine, string | null>();
  const fold = (line: Extract<ThreadLine, { kind: "tool" }>, fields: AcpToolCallFields) => {
    if (fields.title !== undefined) line.title = fields.title;
    if (fields.kind !== undefined) line.toolKind = fields.kind;
    if (fields.status !== undefined) line.status = fields.status;
    if (fields.rawInput !== undefined) line.input = commandOf(fields.rawInput);
    if (fields.rawInput !== undefined || fields.locations !== undefined) line.subject = subjectOf(fields.rawInput, fields.locations) ?? line.subject;
    if (fields.content !== undefined) {
      line.diffs = fields.content.filter((item): item is Extract<AcpToolCallContent, { type: "diff" }> => item.type === "diff");
      const blocks = fields.content.flatMap((item) => (item.type === "content" ? [item.content] : []));
      const words = blocks.map(textOf).filter(Boolean).join("\n");
      line.output = words || null;
      line.images = blocks.filter((block) => block.type === "image");
    }
  };
  updates.forEach((update, index) => {
    // A message line is named by its first update: its lasting name where the caller has one, else its place.
    const id = `u${ids[index] ?? index}`;
    const last = lines.at(-1);
    const message = "messageId" in update ? (update.messageId ?? null) : null;
    switch (update.sessionUpdate) {
      case "user_message_chunk": {
        const origin = update._meta?.hide?.origin ?? "operator";
        const from = update._meta?.hide?.from ?? null;
        if (last?.kind === "user" && last.origin === origin && last.message === message && (senders.get(last) ?? null) === from) {
          last.text += textOf(update.content);
          if (update.content.type === "image") last.images.push(update.content);
        } else {
          const hide = update._meta?.hide;
          const line: ThreadLine = { kind: "user", id, message, text: textOf(update.content), images: update.content.type === "image" ? [update.content] : [], origin, peer: from === null ? null : session(from), letterKind: letterKindOf(hide?.kind), summary: hide?.summary ?? null };
          senders.set(line, from);
          lines.push(line);
        }
        return;
      }
      case "agent_message_chunk":
      case "agent_thought_chunk": {
        if (update.sessionUpdate === "agent_thought_chunk") {
          if (last?.kind === "thought" && last.message === message) last.text += textOf(update.content);
          else lines.push({ kind: "thought", id, message, text: textOf(update.content) });
          return;
        }
        if (last?.kind === "agent" && last.message === message) last.text += textOf(update.content);
        else lines.push({ kind: "agent", id, message, text: textOf(update.content), atMs: update._meta?.hide?.at_ms ?? null });
        return;
      }
      case "tool_call": {
        const part = parts[update.toolCallId];
        if (part) {
          // Every reader titles a shell call by its command, whatever input it keeps (Codex's `cmd`, Antigravity's `CommandLine`, none at all).
          lines.push({ kind: "hide", id: update.toolCallId, part, command: commandOf(update.rawInput) ?? (update.title || null) });
          return;
        }
        const asked = update._meta?.hide?.question;
        if (asked?.length) {
          const line: Extract<ThreadLine, { kind: "question" }> = { kind: "question", id: update.toolCallId, questions: asked.map((question) => ({ text: question.text, choices: question.choices, answer: null })), status: "pending" };
          answer(line, update);
          questions.set(update.toolCallId, line);
          lines.push(line);
          return;
        }
        const line: Extract<ThreadLine, { kind: "tool" }> = { kind: "tool", id: update.toolCallId, title: update.title, toolKind: "other", status: "pending", input: null, subject: null, output: null, diffs: [], images: [], startedAtMs: update._meta?.hide?.at_ms ?? null };
        fold(line, update);
        calls.set(update.toolCallId, line);
        lines.push(line);
        return;
      }
      case "tool_call_update": {
        const asked = questions.get(update.toolCallId);
        if (asked) answer(asked, update);
        const line = calls.get(update.toolCallId);
        if (line) fold(line, update);
        return;
      }
      case "plan":
        lines.push({ kind: "plan", id, entries: update.entries });
        return;
    }
  });
  return lines;
}

// ---------- rules ----------

/** The kinds a session's View list contributes to 결과물 (D-65, B30); code, diffs, logs and Hide AI's names stay out. */
export function outputKindOf(kind: SessionViewKind): DeskOutputKind | null {
  switch (kind) {
    case "image":
      return "image";
    case "page":
      return "page";
    case "pr":
      return "pr";
    case "markdown":
    case "pdf":
    case "html":
      return "doc";
    case "code":
    case "diff":
    case "log":
    case "hide_ai":
      return null;
  }
}

/** The sessions Desk started that ask the operator: the sidebar Desk row's warning mark (PRD desk-2-ui B46). */
export function askingCount(jobs: readonly DeskJob[]): number {
  return jobs.filter((job) => job.activity === "asking").length;
}

/**
 * A message split at its mentions (B18): each mention's own token in the
 * text (`@session(<id> <name>)` for a Desk message, `@name` in a session's
 * words) becomes the mention, the longest token first, so a name with spaces
 * or a path in it is read whole.
 */
export function mentionParts(text: string, mentions: readonly DeskMention[], token: (mention: DeskMention) => string): (string | DeskMention)[] {
  const tokens = mentions.map((mention) => ({ mention, token: token(mention) })).sort((a, b) => b.token.length - a.token.length);
  const parts: (string | DeskMention)[] = [];
  let plain = "";
  let at = 0;
  while (at < text.length) {
    const hit = text[at] === "@" ? tokens.find((one) => text.startsWith(one.token, at)) : undefined;
    if (!hit) {
      plain += text[at];
      at += 1;
      continue;
    }
    if (plain) parts.push(plain);
    plain = "";
    parts.push(hit.mention);
    at += hit.token.length;
  }
  if (plain) parts.push(plain);
  return parts;
}

/**
 * What the working line says an agent does now (feedback 1 c3): a short word
 * from a closed set, Desk's steps by the names `DeskSteps` counts them, and a
 * session's by its newest line.
 */
export type WorkingPhase = "thinking" | "answering" | "editing" | DeskStepKind;

/**
 * Whether Desk's conversation keeps its foot in view after a scroll from `before` to `now` (scrollTop): reaching
 * the foot holds it, and only a move up away from it lets go. A scroll the browser makes itself while the
 * content grows (scroll anchoring moves it down) arrives before the resize that pins the foot, so a far foot
 * alone never lets go.
 */
export function holdsFoot(held: boolean, before: number, now: number, distance: number, near: number): boolean {
  if (distance < near) return true;
  return now < before ? false : held;
}

/** Desk's turn as the working line says it: the step its newest call does, else answering once it has written, else thinking. */
export function deskPhase(step: DeskStepKind | null | undefined, text: string | null): WorkingPhase {
  return step ?? (text ? "answering" : "thinking");
}

/** A mission mention as a Desk answer's text holds it. */
export const markedMission = (mission: DeskMissionMention) => `@mission(${mission.id} ${mission.title})`;

/** A mention as a Desk message's text holds it. */
export const markedMention = (mention: DeskMention) => `@${mention.kind}(${mention.id} ${mention.name})`;

/** A mention as a session's own words say it. */
export const namedMention = (mention: DeskMention) => `@${mention.name}`;

/** An agent Desk cannot be given (B4, D-79): one Settings › Agents switched off. One not ready or not signed in can be picked and says so (B6). */
export function agentPickable(agent: DeskAgent): boolean {
  return agent.state !== "off";
}

/** What the agent bar says beside an agent (D-81, B44): a ready agent's sign-in as Settings › Agents reads it, nothing while that is unread; otherwise its state. */
export type DeskAgentWord = Exclude<DeskAgentState, "ready"> | "signed_in" | "signed_out";

export function deskAgentWord(agent: DeskAgent): DeskAgentWord | null {
  if (agent.state !== "ready") return agent.state;
  if (agent.signed_in === null) return null;
  return agent.signed_in ? "signed_in" : "signed_out";
}

/** A turn as a question Hide found looks in it: every piece of the agent's words, in order (`ConversationTurn`). */
type WordsOfTurn = { texts: readonly Extract<ThreadLine, { kind: "agent" }>[] };

/** The agent's words written no later than `atUnixMs`, newest first, the later in the conversation first among lines of one time. */
function answersBy(turns: readonly WordsOfTurn[], atUnixMs: number): Extract<ThreadLine, { kind: "agent" }>[] {
  return turns
    .flatMap((turn) => turn.texts)
    .filter((line) => line.atMs !== null && line.atMs <= atUnixMs)
    .reverse()
    .sort((a, b) => (b.atMs ?? 0) - (a.atMs ?? 0));
}

/** A place in a message: where the sentence a question asks stands in it, or the whole message. */
export type AskSentence = { from: number; to: number };

/**
 * Whether the question line can take the operator to where a question was
 * asked (Doop N3 가), the one place that decides it: `found`, with the message
 * and what of it to light; `unknown` while the record is being read or holds
 * pages before the ones read, which a press reads first; `gone` once every
 * page is read and no answer was written by the time Hide found the question
 * (the agent started a new record, or the record was cut short), a row a
 * press does nothing on.
 *
 * Time decides whether the message is there: the agent's words written no
 * later than the question was found (`_meta.hide.at_ms`), every piece of
 * them, since none is folded away. The agent's own
 * sentence only decides what of it to light: the newest of them that holds
 * the sentence, lit at the sentence, since the core checked it against the
 * turn's answer messages together; when none holds it as the core read it (a
 * reader that writes the message a little differently), the newest of them,
 * lit whole. A question kept before its sentence was goes to the newest of
 * them with nothing lit.
 */
export function askPlace(turns: readonly WordsOfTurn[], ask: { at_unix_ms: number; quote?: string }, { olderLeft, settled }: { olderLeft: boolean; settled: boolean }):
  | { state: "found"; line: Extract<ThreadLine, { kind: "agent" }>; sentence: AskSentence | null }
  | { state: "unknown" | "gone" } {
  const answers = answersBy(turns, ask.at_unix_ms);
  const newest = answers[0];
  if (newest) {
    if (!ask.quote) return { state: "found", line: newest, sentence: null };
    for (const line of answers) {
      const sentence = sentenceIn(line.text, ask.quote);
      if (sentence) return { state: "found", line, sentence };
    }
    return { state: "found", line: newest, sentence: wholeOf(newest.text) };
  }
  return { state: olderLeft || !settled ? "unknown" : "gone" };
}

/** The whole of a message's text, its white space at either end left out. */
function wholeOf(text: string): AskSentence {
  return { from: text.length - text.trimStart().length, to: text.trimEnd().length };
}
