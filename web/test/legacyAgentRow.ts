// Frozen pre-refactor fixture adapter (main 9f144877).
// Unit tests keep their old input vocabulary and independent assertions while
// exercising renderers of the new wire shape. Never import this in live code.
import type { AgentRow, AgentState, RowMark, StatusTone } from "../src/snapshot";

/** The machine a fixture row is on when it names none. */
export const LOCAL = "local";

type Input = Omit<Partial<AgentRow>, "state"> & { state?: AgentState };
export function legacyAgentRow(agent: Input): AgentRow {
  const asking = ["question", "approval", "error"].includes(agent.demand ?? "none");
  const needs = agent.group === "needs_you";
  const attention = needs || Boolean(agent.unread);
  // The mark the core would draw when a fixture names none (docs/status-model.md, The mark).
  const mark: RowMark = agent.mark ?? (needs || (asking && !agent.delegated) ? "turn" : agent.activity === "working" ? "going" : agent.wait ? "waiting" : agent.group === "done" ? "unread" : "quiet");
  const chipKind = mark === "turn" ? "warning" : mark === "going" || mark === "waiting" ? "working" : mark === "done" ? "success" : mark === "unread" ? "news" : asking ? "warning" : "subtle";
  const chip: StatusTone = { kind: chipKind, read: asking && !needs && !agent.emphasized };
  const markTone: StatusTone = { kind: mark === "quiet" ? "subtle" : chipKind, read: mark === "turn" && chip.read };
  const bucket = needs || agent.group === "done" ? "turn" : agent.wait ? "delegating" : agent.group === "working" ? "working" : "resting";
  const counts = agent.descendant_counts;
  const waits = Boolean(agent.wait || (counts && counts.working + counts.question + counts.approval + counts.error > 0));
  const working = agent.group === "working" || Boolean(agent.wait);
  const text = agent.detail?.trim();
  const mode = asking ? "request" : agent.wait ? "waiting" : agent.unread ? "news" : "quiet";
  const verb = agent.request?.verb ?? (agent.group === "working" ? "working" : "idle");
  const todo = ["answer", "blocked", "fix", "review", "stopped", "result"].includes(verb);
  // The core names a row's machine and own pane id on every row; a fixture
  // row is the `local` machine's unless it names another, and a device
  // fixture that reads the own id names it.
  return { ...agent, device_id: agent.device_id ?? LOCAL, device_pane_id: agent.device_pane_id ?? agent.pane_id, mark, state: {
    attention, needs_you: needs, root: !agent.delegated,
    title_emphasized: !(agent.delegated && !attention) && (attention || Boolean(agent.emphasized)),
    selection_emphasizes_title: !agent.delegated || attention,
    asking, working, waits_on_children: waits,
    chip_tone: chip, mark_tone: markTone,
    line: text ? { text, mode, tone: mode === "request" ? chip : { kind: mode === "news" ? "news" : "subtle", read: false } } : null,
    branch_badge: agent.delegated ? agent.lineage_worktree_badge?.trim() || null : null,
    bucket,
    attention_rank: needs ? agent.demand === "error" ? 0 : 1 : agent.group === "done" ? 2 : agent.group === "working" ? 3 : 4,
    link_rank: asking ? 0 : working ? 1 : 2,
    search_tone: chip.kind === "warning" ? "attention" : chip.kind === "working" ? "working" : chip.kind === "success" ? "done" : "muted",
    subtree: agent.requires_close_status_check ? "unknown" : agent.demand && agent.demand !== "none" ? "waiting" : agent.activity === "working" ? "working" : agent.unread && (mark === "done" || mark === "unread") ? "unread" : "quiet",
    link: asking ? "question" : working ? "working" : "idle",
    descendant_asking: (counts?.question ?? 0) + (counts?.approval ?? 0),
    verb, request_todo: todo, request_since: todo ? agent.request?.verb_since_unix_ms ?? null : agent.changed_at_unix_ms ?? null,
  } } as AgentRow;
}
