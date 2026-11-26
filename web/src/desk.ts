// Desk's screen values (PRD session-docs-and-assistant, Desk 1차): what the
// Desk room, its job cards, the Threads list and a thread draw. The wire's
// own shapes are `snapshot.ts`'s (the `desk`, `desk_live`, `desk_jobs` and
// `desk_mentions` sections), re-exported here under the names the parts use;
// the types below them are the screen's own. The parts take these values as
// props only; nothing here reads the store or the socket.
//
// The fields no section carries (marked `wire gap` below) are a thread's
// own, which the threads node reads from the session record.

import { candidateAgent } from "./conductorSettings";
import { sentenceIn } from "./markdownPlain";
import type { AgentRow, ConductorCandidate, DeskAgent, DeskAgentState, DeskAskKind, DeskItem, DeskOption, DeskOptionKind, DeskJob, DeskJobsSection, DeskRecord, DeskMention as WireMention, DeskMission, DeskMissionMention, DeskNotice, DeskOutput, DeskOutputKind, DeskSection, DeskSessionChip, DeskStepKind, PlanView, PrState, RowMark } from "./snapshot";

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
 * A chip's session as the sidebar and Desk's cards know it now (D-86): a
 * session named by its pane that no row has any more is not listed, so its
 * chip is dimmed; one named only by a name no row carries is not known, and
 * its chip keeps the values it came with.
 */
export function sessionFacts(rows: ReadonlyMap<string, AgentRow>, cards: readonly DeskJob[], session: DeskSessionRef, records: readonly DeskRecord[] = []): SessionFacts | null {
  const row = session.pane_id ? rows.get(session.pane_id) : [...rows.values()].find((one) => one.identity_label === session.name);
  if (!row) {
    if (!session.pane_id) return null;
    const record = records.find((one) => one.pane_id === session.pane_id) ?? null;
    return { listed: false, name: record?.title ?? null, mark: null, agent_kind: record?.agent_kind ?? null, card: record ? recordCard(record) : null, since: record?.ended_at_unix_ms ?? null, parent: null, record };
  }
  const parent = row.lineage_parent_pane_id ? rows.get(row.lineage_parent_pane_id) : undefined;
  return {
    listed: true,
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
    state: "done",
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
 * Hide's own marks under `_meta.hide`: a message's origin and a letter's
 * sender as written, a cut text or image, and when a call or its result was
 * written (Unix milliseconds).
 */
export type AcpMeta = { hide?: { origin?: "letter" | "interrupted"; from?: string; truncated?: boolean; at_ms?: number } } | null;

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
export type ThreadHidePart =
  | { kind: "plan"; plan: PlanView }
  /** A new session this one handed work to. */
  | { kind: "spawn"; session: DeskSessionRef }
  /** A letter this session sent or got, with the other session and its first line. */
  | { kind: "letter"; direction: "sent" | "received"; peer: DeskSessionRef; body: string }
  | { kind: "view"; output: DeskOutput }
  | { kind: "mission"; mission: DeskMission }
  | { kind: "pr"; number: number; state: PrState };

/** One drawn line of a thread, after chunks are joined and call updates folded into their call. */
export type ThreadLine =
  /** `peer` is a letter's sender, when the record names it. */
  | { kind: "user"; id: string; message: string | null; text: string; images: AcpContentBlock[]; origin: "operator" | "letter" | "interrupted"; peer: DeskSessionRef | null }
  /** `atMs` is when its record was written, when the record says (its first piece's time). */
  | { kind: "agent"; id: string; message: string | null; text: string; atMs: number | null }
  | { kind: "thought"; id: string; message: string | null; text: string }
  /** `startedAtMs` is when the session made the call, when its record says. */
  | { kind: "tool"; id: string; title: string; toolKind: AcpToolKind; status: AcpToolCallStatus; input: string | null; output: string | null; diffs: Extract<AcpToolCallContent, { type: "diff" }>[]; images: AcpContentBlock[]; startedAtMs: number | null }
  | { kind: "plan"; id: string; entries: AcpPlanEntry[] }
  /** `command` is what the session typed, when the call carries it. */
  | { kind: "hide"; id: string; part: ThreadHidePart; command: string | null };

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
 * instead (B26). `session` names a letter's sender as its chip draws it.
 */
export function threadLines(updates: readonly AcpSessionUpdate[], parts: Readonly<Record<string, ThreadHidePart>> = {}, session: (name: string) => DeskSessionRef = nameOnly): ThreadLine[] {
  const lines: ThreadLine[] = [];
  const calls = new Map<string, Extract<ThreadLine, { kind: "tool" }>>();
  // A letter's sender as the record wrote it, so its chunks join by who wrote them, not by how the chip names them.
  const senders = new Map<ThreadLine, string | null>();
  const fold = (line: Extract<ThreadLine, { kind: "tool" }>, fields: AcpToolCallFields) => {
    if (fields.title !== undefined) line.title = fields.title;
    if (fields.kind !== undefined) line.toolKind = fields.kind;
    if (fields.status !== undefined) line.status = fields.status;
    if (fields.rawInput !== undefined) line.input = commandOf(fields.rawInput);
    if (fields.content !== undefined) {
      line.diffs = fields.content.filter((item): item is Extract<AcpToolCallContent, { type: "diff" }> => item.type === "diff");
      const blocks = fields.content.flatMap((item) => (item.type === "content" ? [item.content] : []));
      const words = blocks.map(textOf).filter(Boolean).join("\n");
      line.output = words || null;
      line.images = blocks.filter((block) => block.type === "image");
    }
  };
  updates.forEach((update, index) => {
    const id = `u${index}`;
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
          const line: ThreadLine = { kind: "user", id, message, text: textOf(update.content), images: update.content.type === "image" ? [update.content] : [], origin, peer: from === null ? null : session(from) };
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
          lines.push({ kind: "hide", id: update.toolCallId, part, command: commandOf(update.rawInput) });
          return;
        }
        const line: Extract<ThreadLine, { kind: "tool" }> = { kind: "tool", id: update.toolCallId, title: update.title, toolKind: "other", status: "pending", input: null, output: null, diffs: [], images: [], startedAtMs: update._meta?.hide?.at_ms ?? null };
        fold(line, update);
        calls.set(update.toolCallId, line);
        lines.push(line);
        return;
      }
      case "tool_call_update": {
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
  return jobs.filter((job) => job.state === "asking").length;
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

/** A session's running turn as the working line says it, from its newest line: its words, else its newest call or Hide part. */
export function threadPhase(turn: ThreadTurn | undefined): WorkingPhase {
  if (!turn) return "thinking";
  if (turn.answer.length > 0) return "answering";
  const last = turn.work.at(-1);
  if (last?.kind === "tool") {
    switch (last.toolKind) {
      case "read":
      case "search":
      case "fetch":
        return "files";
      case "edit":
      case "delete":
      case "move":
        return "editing";
      case "execute":
        return "other";
      default:
        return "thinking";
    }
  }
  if (last?.kind === "hide") return last.part.kind === "letter" ? "letters" : last.part.kind === "spawn" ? "starts" : "sessions";
  return "thinking";
}

/** A mission mention as a Desk answer's text holds it. */
export const markedMission = (mission: DeskMissionMention) => `@mission(${mission.id} ${mission.title})`;

/** A mention as a Desk message's text holds it. */
export const markedMention = (mention: DeskMention) => `@${mention.kind}(${mention.id} ${mention.name})`;

/** A mention as a session's own words say it. */
export const namedMention = (mention: DeskMention) => `@${mention.name}`;

/** A Desk message's words as the operator reads them, each held mention said by its name: what `복사` puts on the clipboard. */
export function namedText(text: string, mentions: readonly DeskMention[]): string {
  return mentions.reduce((words, mention) => words.split(markedMention(mention)).join(namedMention(mention)), text);
}

/** An agent Desk cannot be given (B4, D-79): one Settings › Agents switched off. One not ready or not signed in can be picked and says so (B6). */
export function agentPickable(agent: DeskAgent): boolean {
  return agent.state !== "off";
}

/** A favorite model (Settings › Agents) as the agent menu offers it: one pick that sets the agent, its model and its effort together. */
export type DeskFavorite = { key: string; agent: DeskAgent; model: string | null; effort: string | null; modelLabel: string; purpose: string };

/**
 * The favorite models Desk's agent menu lists first. A favorite is listed
 * when Desk can start what it names: its agent is one the bar offers, it
 * names no profile (Desk runs an agent's built-in setup), and its model and
 * effort are ones the agent's last session offered. Before that session the
 * agent has offered none, and a pick that switches to the agent is applied
 * as it opens, so such a favorite is listed only while another agent is
 * chosen.
 */
export function deskFavorites(candidates: readonly ConductorCandidate[], agents: readonly DeskAgent[], selection: DeskSection["selection"]): DeskFavorite[] {
  const favorites: DeskFavorite[] = [];
  candidates.forEach((candidate, index) => {
    if (candidate.profile) return;
    const kind = candidateAgent(candidate.agent);
    const agent = agents.find((one) => one.id === kind);
    if (!agent || !agentPickable(agent)) return;
    const model = candidate.model ?? null;
    const effort = candidate.effort ?? null;
    const offered = (choices: readonly { id: string }[], id: string | null) => id === null || (choices.length === 0 ? selection.agent !== agent.id : choices.some((one) => one.id === id));
    if (!offered(agent.models, model) || !offered(agent.efforts, effort)) return;
    const modelLabel = model === null ? "" : (agent.models.find((one) => one.id === model)?.label ?? model);
    favorites.push({ key: `${index}`, agent, model, effort, modelLabel, purpose: candidate.purpose });
  });
  return favorites;
}

/** Whether a favorite is what Desk's next message goes to now. */
export function favoriteChosen(favorite: DeskFavorite, selection: DeskSection["selection"]): boolean {
  return selection.agent === favorite.agent.id && (favorite.model === null || selection.model === favorite.model) && (favorite.effort === null || selection.effort === favorite.effort);
}

/** What the agent bar says beside an agent (D-81, B44): a ready agent's sign-in as Settings › Agents reads it, nothing while that is unread; otherwise its state. */
export type DeskAgentWord = Exclude<DeskAgentState, "ready"> | "signed_in" | "signed_out";

export function deskAgentWord(agent: DeskAgent): DeskAgentWord | null {
  if (agent.state !== "ready") return agent.state;
  if (agent.signed_in === null) return null;
  return agent.signed_in ? "signed_in" : "signed_out";
}

/** One turn of a thread (B25): the words that started it, the work, the answer, and the parts the work made. */
export type ThreadTurn = {
  key: string;
  /** The message that began the turn; the first turn may have none. */
  user: Extract<ThreadLine, { kind: "user" }> | null;
  /** The work before the answer: thoughts, calls, plans and every Hide part. */
  work: ThreadLine[];
  /** The agent's words after its last piece of work. */
  answer: Extract<ThreadLine, { kind: "agent" }>[];
  /** The Hide parts the work made that lead somewhere: they stay in view while the work is folded (B26). */
  parts: Extract<ThreadLine, { kind: "hide" }>[];
  files: number;
  commands: number;
};

/**
 * A thread's lines as turns (B25): each message starts one, and its work
 * folds into one line, `파일 n · 명령 m`, above the answer. A new session, a
 * letter, an output, a mission or a PR it made stays in view under the
 * folded line; a plan change does not.
 */
export function threadTurns(lines: readonly ThreadLine[]): ThreadTurn[] {
  const turns: ThreadTurn[] = [];
  let open: { user: ThreadTurn["user"]; body: ThreadLine[]; key: string } | null = null;
  const close = () => {
    if (!open) return;
    let lastWork = open.body.length - 1;
    while (lastWork >= 0 && open.body[lastWork]!.kind === "agent") lastWork -= 1;
    const work = open.body.slice(0, lastWork + 1);
    const answer = open.body.slice(lastWork + 1) as Extract<ThreadLine, { kind: "agent" }>[];
    const tools = work.filter((line): line is Extract<ThreadLine, { kind: "tool" }> => line.kind === "tool");
    turns.push({
      key: open.key,
      user: open.user,
      work,
      answer,
      parts: work.filter((line): line is Extract<ThreadLine, { kind: "hide" }> => line.kind === "hide" && line.part.kind !== "plan"),
      files: tools.filter((line) => line.toolKind === "edit" || line.toolKind === "delete" || line.toolKind === "move").length,
      commands: tools.filter((line) => line.toolKind === "execute").length,
    });
  };
  for (const line of lines) {
    if (line.kind === "user") {
      close();
      open = { user: line, body: [], key: line.id };
      continue;
    }
    open ??= { user: null, body: [], key: line.id };
    open.body.push(line);
  }
  close();
  return turns;
}

/**
 * The answer a question Hide found at a turn's end was asked in (B4 question
 * 나): the last answer written no later than the question was found, which
 * the question line goes to for a question kept without its sentence. Hide
 * AI lists a question only when the sentence that asks it is in the turn's
 * own answer, so that answer is where it stands; null when no answer of the
 * lines read says when it was written that early.
 */
export function askedIn(turns: readonly ThreadTurn[], atUnixMs: number): Extract<ThreadLine, { kind: "agent" }> | null {
  return answersBy(turns, atUnixMs)[0] ?? null;
}

/** The answer lines written no later than `atUnixMs`, newest first, the later in the conversation first among lines of one time. */
function answersBy(turns: readonly ThreadTurn[], atUnixMs: number): Extract<ThreadLine, { kind: "agent" }>[] {
  return turns
    .flatMap((turn) => turn.answer)
    .filter((line) => line.atMs !== null && line.atMs <= atUnixMs)
    .reverse()
    .sort((a, b) => (b.atMs ?? 0) - (a.atMs ?? 0));
}

/**
 * Where a question Hide found stands in the lines read (Doop N3 가): for one
 * kept with the agent's own sentence, the newest answer line written by the
 * time it was found that holds that sentence, and where in it; the core
 * checked the sentence against the turn's answer messages together, so it can
 * stand in an earlier message of the turn than the last. For one kept without
 * its sentence, the answer `askedIn` gives, with no place in it. Null when no
 * line read holds it.
 */
export function askedAt(turns: readonly ThreadTurn[], ask: { at_unix_ms: number; quote?: string }): { line: Extract<ThreadLine, { kind: "agent" }>; sentence: { from: number; to: number } | null } | null {
  if (!ask.quote) {
    const line = askedIn(turns, ask.at_unix_ms);
    return line && { line, sentence: null };
  }
  for (const line of answersBy(turns, ask.at_unix_ms)) {
    const sentence = sentenceIn(line.text, ask.quote);
    if (sentence) return { line, sentence };
  }
  return null;
}
