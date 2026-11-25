// Desk's screen values (PRD session-docs-and-assistant, Desk 1차): what the
// Desk room, its job cards, the Threads list and a thread draw. The wire's
// own shapes are `snapshot.ts`'s (the `desk`, `desk_live`, `desk_jobs` and
// `desk_mentions` sections), re-exported here under the names the parts use;
// the types below them are the screen's own. The parts take these values as
// props only; nothing here reads the store or the socket.
//
// The fields no section carries (marked `wire gap` below) are a thread's
// own, which the threads node reads from the session record.

import type { AgentRow, DeskAgent, DeskAgentState, DeskAskKind, DeskIrreversible, DeskItem, DeskJob, DeskJobsSection, DeskMention as WireMention, DeskMission, DeskNotice, DeskOutput, DeskOutputKind, DeskPermissionRow, DeskPermissions, DeskSection, DeskSessionChip, PlanView, PrState, RowMark } from "./snapshot";

export type { DeskAgent, DeskAgentId, DeskAgentState, DeskApprovalAnswer, DeskApprovalState, DeskChoice, DeskIrreversible, DeskItem, DeskJob, DeskJobFailure, DeskJobState, DeskLive, DeskLocation, DeskMission, DeskOutput, DeskOutputKind, DeskPermissionAnswer, DeskQueuedMessage, DeskPermissionRow, DeskPermissions, DeskSteps } from "./snapshot";

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
 * card (D-86): whether the sidebar still lists it, its row's mark and agent,
 * its card (the facts its hover card shows and the thread a press opens),
 * the time its row shows and its parent.
 */
export type SessionFacts = {
  listed: boolean;
  mark: RowMark | null;
  agent_kind: string | null;
  card: DeskJob | null;
  /** When its row's time counts from (`last_prompt_at_unix_ms`). */
  since: number | null;
  parent: DeskSessionRef | null;
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
export function sessionFacts(rows: ReadonlyMap<string, AgentRow>, cards: readonly DeskJob[], session: DeskSessionRef): SessionFacts | null {
  const row = session.pane_id ? rows.get(session.pane_id) : [...rows.values()].find((one) => one.identity_label === session.name);
  if (!row) return session.pane_id ? { listed: false, mark: null, agent_kind: null, card: null, since: null, parent: null } : null;
  const parent = row.lineage_parent_pane_id ? rows.get(row.lineage_parent_pane_id) : undefined;
  return {
    listed: true,
    mark: row.mark,
    agent_kind: row.agent_kind,
    card: cardOfSession(cards, { ...session, pane_id: row.pane_id }),
    since: row.last_prompt_at_unix_ms ?? null,
    parent: parent ? rowSession(parent) : null,
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

/**
 * The irreversible kinds a card asks about, in the core's order, each with
 * what it is about: the core names one subject, the first kind's that has
 * one (an archived session's title, a path outside every project it
 * deletes or overwrites).
 */
export function approvalIrreversible(item: DeskApproval): { kind: DeskIrreversible; subject: string | null }[] {
  let subject = item.subject;
  return item.asks.flatMap((ask) => {
    if (ask.kind !== "irreversible") return [];
    const takes = ask.value === "end_working_session" || ask.value === "delete_outside" || ask.value === "overwrite_outside";
    const own = takes ? subject : null;
    if (takes) subject = null;
    return [{ kind: ask.value, subject: own }];
  });
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

/** A mention as a Desk message's text holds it. */
export const markedMention = (mention: DeskMention) => `@${mention.kind}(${mention.id} ${mention.name})`;

/** A mention as a session's own words say it. */
export const namedMention = (mention: DeskMention) => `@${mention.name}`;

/** A Desk message's words as the operator reads them, each held mention said by its name: what `복사` puts on the clipboard. */
export function namedText(text: string, mentions: readonly DeskMention[]): string {
  return mentions.reduce((words, mention) => words.split(markedMention(mention)).join(namedMention(mention)), text);
}

/** An agent Desk cannot be given (B4, D-79): one that cannot ask before acting, or one Settings › Agents switched off. One not ready or not signed in can be picked and says so (B6). */
export function agentPickable(agent: DeskAgent): boolean {
  return agent.state !== "no_ask_mode" && agent.state !== "off";
}

/** The agents a row of Settings › Labs › Desk › 권한 cannot hold (`always_runs`), named on that row; one that cannot be chosen is left out. */
export function agentsRunningUnasked(agents: readonly DeskAgent[], row: DeskPermissionRow): string[] {
  return agents.filter((agent) => agentPickable(agent) && agent.always_runs.includes(row)).map((agent) => agent.label);
}

/**
 * The three levels under Desk's box (feedback 1, frame A), each a setting of
 * the core's permission table: `다 묻기` runs only reading, `고칠 땐 묻기` also
 * `hide` commands, `알아서` (the table's default) every row. What cannot be
 * undone asks whatever the level, since the core never reads it from the table.
 */
export const DESK_PERMISSION_LEVELS = {
  ask_all: { hide: "ask", read: "run", shell: "ask", edit: "ask" },
  ask_changes: { hide: "run", read: "run", shell: "ask", edit: "ask" },
  auto: { hide: "run", read: "run", shell: "run", edit: "run" },
} as const satisfies Record<string, DeskPermissions>;

export type DeskPermissionLevel = keyof typeof DESK_PERMISSION_LEVELS;

export const DESK_PERMISSION_LEVEL_ORDER: readonly DeskPermissionLevel[] = ["ask_all", "ask_changes", "auto"];

/** The level a table is, or null for a mix no level matches (`줄마다 정함`), set row by row in Settings. */
export function deskPermissionLevel(permissions: DeskPermissions): DeskPermissionLevel | null {
  const rows = Object.keys(permissions) as DeskPermissionRow[];
  return DESK_PERMISSION_LEVEL_ORDER.find((level) => rows.every((row) => DESK_PERMISSION_LEVELS[level][row] === permissions[row])) ?? null;
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
 * 나): the last answer written no later than the question was found, whose
 * line id it returns. Hide AI lists a question only when the sentence that
 * asks it is in the turn's own answer, so that answer is where it stands;
 * null when no answer of the lines read says when it was written that early.
 */
export function askedIn(turns: readonly ThreadTurn[], atUnixMs: number): string | null {
  let found: { id: string; at: number } | null = null;
  for (const turn of turns) {
    for (const line of turn.answer) {
      if (line.atMs !== null && line.atMs <= atUnixMs && (!found || line.atMs >= found.at)) found = { id: line.id, at: line.atMs };
    }
  }
  return found?.id ?? null;
}
