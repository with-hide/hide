// Desk's screen values (PRD session-docs-and-assistant, Desk 1차): what the
// Desk room, its job cards, the Threads list and a thread draw, in the shape
// of the `desk` and `desk_live` sections `snapshot.ts` declares on the desk
// node's branch (feat/desk-host), so they can be handed to these parts
// unchanged once it lands. The parts take these values as props only;
// nothing here reads the store or the socket.
//
// The fields that section does not carry (marked `wire gap` below) are a
// thread's own, which the threads node reads from the session record.

import type { CellTone } from "./missions";
import type { PlanView, PrState, RowMark } from "./snapshot";

/** An agent Desk talks to over ACP; its name is the screen's (`desk.agentName.*`). */
export type DeskAgentId = "claude" | "codex" | "pi";

export type DeskAgentState = "checking" | "ready" | "preparing" | "not_installed" | "install_failed" | "node_missing" | "cli_missing" | "login_needed" | "no_ask_mode";

export type DeskChoice = { id: string; label: string };

export type DeskAgent = {
  id: DeskAgentId;
  label: string;
  state: DeskAgentState;
  /** What the state names: the Node version found, the install's last line, the missing CLI. */
  detail: string | null;
  /** Shown with `login_needed`. */
  login_command: string;
  /** What the agent's last session offered; empty until one opened. */
  models: DeskChoice[];
  efforts: DeskChoice[];
};

export type DeskSelection = { agent: DeskAgentId | null; model: string | null; effort: string | null };

export type DeskMention = {
  kind: "session" | "project";
  id: string;
  name: string;
  /** A session's status and agent, so its chip reads as a session does everywhere. wire gap */
  mark?: RowMark | null;
  agent_kind?: string | null;
};

export type DeskNoticeKind = "blocked" | "preparing" | "host_stopped" | "resume_failed" | "login_needed" | "install_failed" | "no_ask_mode" | "turn_failed";

export type DeskApprovalState = "waiting" | "allowed" | "rejected" | "closed";

export type DeskLocation = "desk" | "worktree" | "checkout";

/** One line of the Desk conversation. A message's text holds a mention as `@session(<id> <name>)` or `@project(<path> <name>)`. */
export type DeskItem =
  | { kind: "user"; id: number; text: string; at_ms: number; mentions: DeskMention[] }
  /** `thought_ms` and `steps` fold the turn into one line: `생각 8초 · 세션 4개 읽음`. */
  | { kind: "reply"; id: number; agent: DeskAgentId; text: string; at_ms: number; state: "done" | "stopped"; thought_ms: number | null; steps: DeskSteps }
  /** How much of the conversation the new agent was handed. */
  | { kind: "switch"; id: number; from: DeskAgentId | null; to: DeskAgentId; carried: "all" | "trimmed" | "none" }
  | { kind: "model"; id: number; model: string | null; effort: string | null }
  | { kind: "approval"; id: number; card: "command" | "edit" | "other"; command: string; folder: string | null; reason: string | null; state: DeskApprovalState }
  /** `detail` is the install's last line, the sign-in command for `login_needed`, or the blocked call. */
  | { kind: "notice"; id: number; notice: DeskNoticeKind; agent: DeskAgentId | null; detail: string | null }
  /** Desk started or queued a job; its card is `jobs`' entry `job_id`. */
  | { kind: "job"; id: number; job_id: string }
  /** A letter Desk sent a running session (B16): handed work when `letter_kind` is `request`; `read` once its runtime took it in. */
  | { kind: "handed"; id: number; agent_id: string; pane_id: string; name: string; project: string | null; letter_kind: string; line: string; letter_id: string | null; read: boolean };

export type DeskJobState = "queued" | "starting" | "working" | "asking" | "done" | "failed" | "cancelled" | "closed" | "moved_out" | "sleeping" | "archived";

/** What an asking job waits on, each said with its own verb (B20): a question, a terminal's permission, a mission's decision, a plan to review, a call that stopped. */
export type DeskAsk = "question" | "approval" | "decision" | "plan_review" | "blocked";

/** What Desk's agent did in a turn, counted from its tool calls. */
export type DeskSteps = { sessions: number; files: number; letters: number; starts: number; other: number };

/** The `desk_live` section: the reply being written and when its turn started, `null` between turns. */
export type DeskLive = { text: string | null; started_at_ms: number | null };

/**
 * A session as a chip names it: its status, its agent and its name (B26); a
 * session Hide does not know the state of has its name alone. `pane_id` is
 * where it runs, the place pressing its chip goes, when Hide knows it.
 */
export type DeskSessionRef = { name: string; mark: RowMark | null; agent_kind: string | null; pane_id?: string | null };

/** What a thread made, as the session's View list holds it (D-65): only these kinds are shown. */
export type DeskOutputKind = "image" | "page" | "doc" | "pr";

export type DeskOutput = {
  id: string;
  kind: DeskOutputKind;
  label: string;
  /** A picture to draw: an image's bytes or a page's capture, when the window holds one. */
  thumb: string | null;
  /** Looked at since it appeared (the ▣ badge counts the rest). */
  seen: boolean;
  /** A PR output's state, so it reads the same on a card, in a thread and in 결과물 (one PR, one state). */
  pr_state: PrState | null;
};

/** One View of a session, before Desk keeps only its outputs (B30). */
export type SessionViewKind = "image" | "page" | "pr" | "markdown" | "pdf" | "html" | "code" | "diff" | "log" | "hide_ai";

export type DeskPullRequest = { number: number; state: PrState };

/** The mission a Conductor thread runs, as its `미션 k/n` chip draws it (B34, D-43); its nodes are not drawn. */
export type DeskMission = { id: string; title: string; status: "planning" | "running" | "merge_wait" | "done" | "cancelled"; done: number; total: number; tone: CellTone; deciding: boolean };

/** One session Desk started or queued; `id` is the spawn's intent, `pane_id` its thread's key. */
export type DeskJob = {
  id: string;
  agent_id: string | null;
  pane_id: string | null;
  title: string;
  agent_kind: string;
  project: { workspace_id: string | null; label: string };
  location: { kind: "checkout" | "worktree"; label: string };
  state: DeskJobState;
  queue_position: number | null;
  now: string | null;
  plan: { done: number; total: number } | null;
  /** The question, the command asking approval, the decision or the plan's step count, as the second line says it. */
  question: string | null;
  summary: string | null;
  pull_requests: DeskPullRequest[];
  outputs: DeskOutput[];
  joined: boolean;
  reason: "spawn_failed" | "exited" | "device_disconnected" | null;
  /** The code a failed start ended with. */
  failure: string | null;
  ask: DeskAsk | null;
  /** The sidebar row's mark, drawn with `StatusMark`; `mark_read` is its hollow read ask. A job with no row leaves it empty. */
  mark: RowMark | null;
  mark_read: boolean;
  machine: string | null;
  disconnected: boolean;
  changed_at_unix_ms: number;
  read: boolean;
  mission: DeskMission | null;
  /** The panes of a Conductor's workers. */
  workers: string[];
};

export type DeskSettings = { default_agent: DeskAgentId | null; location: DeskLocation; cap: number };

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
export type AcpMeta = { hide?: { origin?: "letter" | "interrupted" | "injected"; from?: string; truncated?: boolean; at_ms?: number } } | null;

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
  | { kind: "user"; id: string; message: string | null; text: string; images: AcpContentBlock[]; origin: "operator" | "letter" | "interrupted" | "injected"; peer: DeskSessionRef | null }
  | { kind: "agent"; id: string; message: string | null; text: string }
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
        if (last?.kind === "user" && last.origin === origin && last.message === message && (last.peer?.name ?? null) === from) {
          last.text += textOf(update.content);
          if (update.content.type === "image") last.images.push(update.content);
        } else {
          lines.push({ kind: "user", id, message, text: textOf(update.content), images: update.content.type === "image" ? [update.content] : [], origin, peer: from === null ? null : session(from) });
        }
        return;
      }
      case "agent_message_chunk":
      case "agent_thought_chunk": {
        const kind = update.sessionUpdate === "agent_message_chunk" ? "agent" : "thought";
        if (last?.kind === kind && last.message === message) last.text += textOf(update.content);
        else lines.push({ kind, id, message, text: textOf(update.content) });
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

export type ThreadGroupKey = "turn" | "working" | "done";

/** Which Threads group a job is listed in (B28): what waits on the operator, what runs, what ended. */
export function threadGroupOf(job: DeskJob): ThreadGroupKey | null {
  switch (job.state) {
    case "asking":
    case "failed":
      return "turn";
    case "queued":
    case "starting":
    case "working":
      return "working";
    case "done":
    case "sleeping":
    case "cancelled":
    case "closed":
    case "archived":
      return "done";
    case "moved_out":
      return null;
  }
}

export type ThreadGroup = { key: ThreadGroupKey; jobs: DeskJob[] };

/**
 * The Threads list (B28): the groups in their fixed order, each only while it
 * holds a thread; a mission's workers are listed under their Conductor, never
 * at the top level, and a job that left Desk is not listed.
 */
export function threadGroups(jobs: readonly DeskJob[]): ThreadGroup[] {
  const workers = new Set(jobs.flatMap((job) => job.workers));
  const groups: ThreadGroup[] = [
    { key: "turn", jobs: [] },
    { key: "working", jobs: [] },
    { key: "done", jobs: [] },
  ];
  for (const job of jobs) {
    if (job.pane_id !== null && workers.has(job.pane_id)) continue;
    const key = threadGroupOf(job);
    if (key) groups.find((group) => group.key === key)!.jobs.push(job);
  }
  return groups.filter((group) => group.jobs.length > 0);
}

/** The ☰ badge: threads asking the operator (B28). */
export function askingCount(jobs: readonly DeskJob[]): number {
  return jobs.filter((job) => job.state === "asking").length;
}

/** The ▣ badge: outputs not looked at yet (B31). */
export function unseenOutputs(jobs: readonly DeskJob[]): number {
  return jobs.reduce((sum, job) => sum + job.outputs.filter((output) => !output.seen).length, 0);
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

/** A mention as a Desk message's text holds it. */
export const markedMention = (mention: DeskMention) => `@${mention.kind}(${mention.id} ${mention.name})`;

/** A mention as a session's own words say it. */
export const namedMention = (mention: DeskMention) => `@${mention.name}`;

/** An agent Desk cannot be given (D-53, B4): one that cannot ask before acting. One not installed or signed in can be picked and says so (B6). */
export function agentPickable(agent: DeskAgent): boolean {
  return agent.state !== "no_ask_mode";
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
 * The turns with the question an asking job waits on said once (D-75): the
 * wait at the thread's foot says it, so the last answer leaves it out, whole
 * when the answer is only the question and its ending when it ends with it.
 */
export function askedOnce(turns: readonly ThreadTurn[], question: string | null): readonly ThreadTurn[] {
  const asked = question?.trim();
  const last = turns.at(-1);
  const answer = last?.answer.at(-1);
  if (!asked || !last || !answer) return turns;
  const said = answer.text.trim();
  const only = said.replace(/\s+/g, " ") === asked.replace(/\s+/g, " ");
  if (!only && !said.endsWith(asked)) return turns;
  const rest = only ? "" : said.slice(0, said.length - asked.length).trimEnd();
  const answers = rest ? [...last.answer.slice(0, -1), { ...answer, text: rest }] : last.answer.slice(0, -1);
  return [...turns.slice(0, -1), { ...last, answer: answers }];
}
