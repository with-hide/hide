// Desk's screen values (PRD session-docs-and-assistant, Desk 1차): what the
// Desk room, its job cards, the Threads list and a thread draw, in the shape
// of the snapshot wire the desk node proposed (`desk` rest section: agents,
// host, conversation items, jobs) so that section can be handed to these
// parts unchanged once it is generated into `snapshot.ts`. The parts take
// these values as props only; nothing here reads the store or the socket.
//
// The fields the draft does not name yet (marked `wire gap` below) are what
// the cards and the list need to draw B24, B28 and B34; the desk node adds
// them or the parts drop them.

import type { PlanView, PrState, RowMark } from "./snapshot";

export type DeskAgentState = "ready" | "preparing" | "install_failed" | "node_missing" | "login_needed" | "not_installed" | "no_ask_mode";

export type DeskAgent = {
  id: string;
  label: string;
  state: DeskAgentState;
  /** A code for the failure, worded by the screen (`desk.reason.*`). */
  reason?: string | null;
  login_command?: string | null;
  models: { id: string; label: string }[];
  efforts: string[];
};

export type DeskSelection = { agent: string; model: string; effort: string };

export type DeskMention = { kind: "session" | "project"; name: string; id: string };

export type DeskNoticeKind = "blocked" | "preparing" | "host_stopped" | "resume_failed" | "login_needed" | "install_failed" | "no_ask_mode" | "turn_failed";

export type DeskApprovalState = "waiting" | "allowed" | "rejected" | "closed";

export type DeskItem =
  | { kind: "user"; id: string; text: string; at: number; mentions: DeskMention[] }
  | { kind: "reply"; id: string; text: string; at: number; state: "done" | "stopped" }
  /** How much of the conversation the new agent was handed. */
  | { kind: "switch"; id: string; from: string; to: string; carried: "all" | "trimmed" | "none" }
  | { kind: "model"; id: string; model: string; effort: string }
  | { kind: "approval"; id: string; title: string; command: string; folder: string; reason: string; state: DeskApprovalState }
  /** `detail` is the install's last line, the sign-in command for `login_needed`, or the blocked call. */
  | { kind: "notice"; id: string; notice: DeskNoticeKind; agent: string | null; detail: string | null }
  | { kind: "job"; id: string; job_id: string }
  | { kind: "handed"; id: string; agent_id: string; name: string; project: string };

export type DeskJobState = "queued" | "starting" | "working" | "asking" | "done" | "failed" | "cancelled" | "closed" | "moved_out" | "archived";

/** What a thread made, as the session's View list holds it (D-65): only these kinds are shown. */
export type DeskOutputKind = "image" | "page" | "doc" | "pr";

export type DeskOutput = {
  id: string;
  kind: DeskOutputKind;
  label: string;
  /** A picture to draw: an image's bytes or a page's capture, when the window holds one. */
  thumb?: string | null;
  /** Looked at since it appeared (the ▣ badge counts the rest). */
  seen: boolean;
  /** A PR output's state, so it reads the same on a card, in a thread and in 결과물 (one PR, one state). wire gap */
  pr_state?: PrState | null;
};

/** One View of a session, before Desk keeps only its outputs (B30). */
export type SessionViewKind = "image" | "page" | "pr" | "markdown" | "pdf" | "html" | "code" | "diff" | "log" | "hide_ai";

export type DeskPullRequest = { number: number; state: PrState };

/** A mission node as the Conductor's card and row show it (B34). wire gap */
export type DeskMissionNode = { key: string; title: string; tone: "done" | "review" | "working" | "hold" | "failed" | "waiting" | "cancelled" };

/** The mission a Conductor thread runs (B34). wire gap */
export type DeskMission = { id: string; title: string; status: "planning" | "running" | "merge_wait" | "done" | "cancelled"; done: number; total: number; tone: DeskMissionNode["tone"]; deciding: boolean; nodes: DeskMissionNode[] };

export type DeskJob = {
  id: string;
  agent_id?: string | null;
  pane_id?: string | null;
  title: string;
  project: { workspace_id: string; label: string };
  location: { kind: "checkout" | "worktree"; label: string };
  state: DeskJobState;
  queue_position?: number | null;
  now?: string | null;
  plan?: { done: number; total: number } | null;
  question?: string | null;
  summary?: string | null;
  pull_requests: DeskPullRequest[];
  outputs: DeskOutput[];
  joined: boolean;
  /** A code (`desk.jobReason.*`): spawn_failed, exited, device_disconnected. */
  reason?: string | null;
  machine?: string | null;
  disconnected: boolean;
  /** The agent's runtime, for its mark in the Threads list. wire gap */
  agent_kind?: string | null;
  /** When the state last changed, for the list's time. wire gap */
  changed_at_unix_ms?: number | null;
  /** The operator has read the finished turn. wire gap */
  read?: boolean;
  /** The mission this thread conducts, and its workers' job ids. wire gap */
  mission?: DeskMission | null;
  workers?: string[];
};

export type DeskSettings = { default_agent: string; location: "desk" | "worktree" | "checkout"; cap: number };

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
  | { kind: "letter"; to: string; body: string }
  | { kind: "view"; output: DeskOutput }
  | { kind: "mission"; mission: DeskMission };

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
  const workers = new Set(jobs.flatMap((job) => job.workers ?? []));
  const groups: ThreadGroup[] = [
    { key: "turn", jobs: [] },
    { key: "working", jobs: [] },
    { key: "done", jobs: [] },
  ];
  for (const job of jobs) {
    if (workers.has(job.id)) continue;
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

/** A job's status mark (docs/status-model.md): whose move is next; an ended job keeps the slot empty. */
export function jobMark(job: DeskJob): RowMark | null {
  switch (job.state) {
    case "queued":
    case "starting":
      return "waiting";
    case "working":
      return "going";
    case "asking":
      return "turn";
    case "done":
      return job.read ? "quiet" : "done";
    case "failed":
    case "cancelled":
    case "closed":
    case "moved_out":
    case "archived":
      return null;
  }
}

/** An agent Desk cannot be given (D-53, B4): one that cannot ask before acting. One not installed or signed in can be picked and says so (B6). */
export function agentPickable(agent: DeskAgent): boolean {
  return agent.state !== "no_ask_mode";
}
