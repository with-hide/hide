// Desk's screen values (PRD session-docs-and-assistant, Desk 1차): what the
// Desk room, its job cards, the Threads list and a thread draw. The wire's
// own shapes are `snapshot.ts`'s (the `desk`, `desk_live`, `desk_jobs` and
// `desk_mentions` sections), re-exported here under the names the parts use;
// the types below them are the screen's own. The parts take these values as
// props only; nothing here reads the store or the socket.
//
// The fields no section carries (marked `wire gap` below) are a thread's
// own, which the threads node reads from the session record.

import type { DeskAgent, DeskAskKind, DeskItem, DeskJob, DeskMention as WireMention, DeskMission, DeskNotice, DeskOutput, DeskOutputKind, DeskSection, DeskSessionChip, PlanView, PrState, RowMark } from "./snapshot";

export type { DeskAgent, DeskAgentId, DeskAgentState, DeskApprovalState, DeskChoice, DeskItem, DeskJob, DeskJobFailure, DeskJobState, DeskLive, DeskLocation, DeskMission, DeskOutput, DeskOutputKind, DeskSteps } from "./snapshot";

export type DeskSelection = DeskSection["selection"];
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

/** A session as a chip names it: its status, its agent and its name (B26); a session Hide does not know the state of has its name alone. */
export type DeskSessionRef = { name: string; mark: RowMark | null; agent_kind: string | null };

/** The session a handed line names, from the line's own mark and agent. */
export function handedSession(item: Extract<DeskItem, { kind: "handed" }>): DeskSessionRef {
  return { name: item.name, mark: item.mark, agent_kind: item.agent_kind };
}

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

/** Hide's own marks under `_meta.hide`: a message's origin, a cut text or image. */
export type AcpMeta = { hide?: { origin?: "letter" | "interrupted" | "injected"; truncated?: boolean } } | null;

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
  | { kind: "spawn"; job: DeskJob }
  /** A letter this session sent or got, with the other session and its first line. */
  | { kind: "letter"; direction: "sent" | "received"; peer: DeskSessionRef; body: string }
  | { kind: "view"; output: DeskOutput }
  | { kind: "mission"; mission: DeskMission }
  | { kind: "pr"; number: number; state: PrState };

/** One drawn line of a thread, after chunks are joined and call updates folded into their call. */
export type ThreadLine =
  | { kind: "user"; id: string; message: string | null; text: string; images: AcpContentBlock[]; origin: "operator" | "letter" | "interrupted" | "injected" }
  | { kind: "agent"; id: string; message: string | null; text: string }
  | { kind: "thought"; id: string; message: string | null; text: string }
  | { kind: "tool"; id: string; title: string; toolKind: AcpToolKind; status: AcpToolCallStatus; input: string | null; output: string | null; diffs: Extract<AcpToolCallContent, { type: "diff" }>[]; images: AcpContentBlock[] }
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

/**
 * The lines a thread draws (B25): consecutive chunks of one kind and one
 * message joined (a chunk with no message id joins the line before it), a call's updates folded into the call (an update's content replaces
 * the call's, as ACP says), and a call the reader turned into a Hide part
 * drawn as that part instead (B26).
 */
export function threadLines(updates: readonly AcpSessionUpdate[], parts: Readonly<Record<string, ThreadHidePart>> = {}): ThreadLine[] {
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
        if (last?.kind === "user" && last.origin === origin && last.message === message) {
          last.text += textOf(update.content);
          if (update.content.type === "image") last.images.push(update.content);
        } else {
          lines.push({ kind: "user", id, message, text: textOf(update.content), images: update.content.type === "image" ? [update.content] : [], origin });
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
        const line: Extract<ThreadLine, { kind: "tool" }> = { kind: "tool", id: update.toolCallId, title: update.title, toolKind: "other", status: "pending", input: null, output: null, diffs: [], images: [] };
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
  return agent.state !== "no_ask_mode" && agent.state !== "not_offered";
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
