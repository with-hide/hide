// What an agent row shows, as pure rules over the core's row (PRD
// sidebar-agent-status D-05, B7-B9). The core decides the group, the mark, the
// read axis, the waiting state and the sentence; these rules decide only when
// a view draws the sentence and how the mark and badge are coloured, so the
// sidebar and any other list of agents (the Overview) cannot disagree.

import { agentProductName } from "./agentAdapters";
import { hideAiWords } from "./hideAiOrigin";
import type { TFunction } from "i18next";
import { statusText } from "./agentStatus";
import { toneClass } from "./lineage";
import { askSubjectText } from "./askSubject";
import type { AgentRow, AgentStatusCode, AskSubject, AgentWait, LineMode, MarkCounts, RowMark, TabAgent } from "./snapshot";

/**
 * Why a row's second line is on screen at rest (the core's `RowLine::mode`).
 *
 * - `request`: the agent is asking the operator (question, approval) or is blocked;
 *   the line stays until the request is resolved, reading it does not end it.
 * - `news`: the row changed since the operator last looked; it goes once read.
 * - `waiting`: an in-progress row's sentence, or with no text its proven cause.
 * - `kept`: the work left or the unproven wait of a read stop, at rest and subdued.
 * - `quiet`: nothing to report; the line shows only on the selected or hovered
 *   row, where it is revealed in full.
 * - `vanished`: the work the agent was waiting for died with its process.
 * - `checks_passed`, `checks_failed`: the checks the row waited on ended.
 * The modes with no text are worded here, in the operator's language.
 */
export type { LineMode };

export type RowLine = { text: string; mode: LineMode; pull_request?: number; subject?: AskSubject };

/** The row's second line and why it shows, or null when the core gave it none. */
export function rowLine(agent: Pick<AgentRow, "state">): RowLine | null {
  const line = agent.state.line;
  return line ? { text: line.text, mode: line.mode, pull_request: line.pull_request, subject: line.subject } : null;
}

/**
 * The line a sidebar row draws (PRD sidebar-readability D-4, B4, B6): anything
 * but a quiet sentence, on one line from the moment it exists, so a pointer,
 * the keyboard or a selection never adds a line or grows the row. A quiet
 * sentence is read in the row's tooltip. A request with no sentence still
 * draws its word (B5). A sleeping row has no line, its moon and hover say it;
 * a waking or failed one says so on its line (status-marks B31).
 */
export function sidebarLine(agent: Pick<AgentRow, "state" | "status_code" | "mark">): RowLine | null {
  const own = stateLine(agent);
  if (own !== undefined) return own;
  const line = rowLine(agent);
  if (line && line.mode !== "quiet") return line;
  return agent.state.asking ? { text: "", mode: "request" } : null;
}

/**
 * The line an agent's own state decides in every list, ahead of its
 * sentence: a sleeping row has none, its moon and hover say it, unless the
 * operator's turn outranks the moon (D-10) and its ask reads as any other; a
 * waking, a failed and an unknown row say their state (B16, B31). Undefined
 * when the state decides nothing.
 */
export function stateLine(agent: Pick<AgentRow, "status_code" | "mark">): RowLine | null | undefined {
  switch (agent.status_code) {
    case "sleeping":
      return agent.mark === "turn" ? undefined : null;
    case "waking":
    case "sleep_failed":
    case "unknown":
      return { text: "", mode: "kept" };
    default:
      return undefined;
  }
}

/**
 * A tree row's line: a tree row is one line (B14), except a block's cause
 * (agent-blocked-state B2), a dimmed row's reason (B16), a waiting row's 대기
 * (status-marks B29) and a waking or failed sleep (B31).
 */
export function treeLine(agent: Pick<AgentRow, "state" | "status_code" | "mark">): RowLine | null {
  const line = sidebarLine(agent);
  if (!line) return null;
  return agent.status_code === "error" || stateLine(agent) || line.mode === "waiting" ? line : null;
}

/** The background tasks a finished turn left, as its line's tail (B27), or null when none. */
export function backgroundTail(t: TFunction<"translation">, agent: { background_left?: number | null }): string | null {
  return agent.background_left ? t("agents.line.backgroundLeft", { count: agent.background_left }) : null;
}

/** Status codes that lead a line as its word: what was asked, how a turn ended (D-04). */
const LEAD_WORDS = new Set<AgentStatusCode>(["error", "question", "approval", "done", "unfinished"]);

/** The word that leads a row's line, or null when its status code is not one. */
export function leadWord(t: TFunction<"translation">, agent: Pick<AgentRow, "status_code">): string | null {
  return LEAD_WORDS.has(agent.status_code) ? statusText(t, agent.status_code) : null;
}

/** What `lineText` reads off a row: a desktop row, or the phone's. */
export type LineAgent = { wait?: AgentWait | null; status_code: AgentStatusCode; descendant_counts?: { working: number } | null; background_left?: number | null };

/**
 * A line in its three parts, so a narrow row cuts only the sentence: the word
 * that leads it, the sentence, and the background tasks a finished turn left
 * (`끝남 · … · 백그라운드 2`, status-marks B27). Empty parts are null.
 */
export type LineParts = { lead: string | null; text: string | null; tail: string | null };

/**
 * The sentence as a row draws it. A request, a read row's news and a kept line
 * lead with their word (`질문 · …`, `일 남음 · …`, D-04); a waiting in-progress
 * row leads with 대기 and says its sentence or, with none, the cause Hide
 * proved (B10, B29); the end of the checks a row waited on names the pull
 * request (B13).
 */
export function lineParts(t: TFunction<"translation">, agent: LineAgent, line: RowLine): LineParts {
  const word = leadWord(t, agent);
  const tail = backgroundTail(t, agent);
  const parts = (lead: string | null, text: string | null): LineParts => ({ lead, text: text || null, tail });
  switch (line.mode) {
    case "vanished":
      return parts(statusText(t, "stopped"), t("agents.line.vanished"));
    case "checks_passed":
      return parts(null, t("agents.line.checksPassed", { number: line.pull_request ?? 0 }));
    case "checks_failed":
      return parts(null, t("agents.line.checksFailed", { number: line.pull_request ?? 0 }));
    case "waiting":
      return parts(statusText(t, "waiting"), line.text || waitCause(t, agent, line));
    case "kept":
      return parts(word ?? statusText(t, agent.status_code), line.text);
    case "request":
      return parts(word, line.subject ? askSubjectText(t, line.subject) : line.text);
    case "news":
      return parts(word, line.text);
    case "quiet":
      return parts(null, line.text);
  }
}

/** The line as one string, its parts joined: what a tooltip, a phone row or a test reads. */
export function lineText(t: TFunction<"translation">, agent: LineAgent, line: RowLine): string {
  const { lead, text, tail } = lineParts(t, agent, line);
  return [lead, text, tail].filter(Boolean).join(" · ");
}

/** What a waiting in-progress row with no sentence is proven to wait on (B10), or null when the cause names nothing past 대기. */
function waitCause(t: TFunction<"translation">, agent: LineAgent, line: RowLine): string | null {
  switch (agent.wait) {
    case "checks":
      return line.pull_request ? t("agents.line.checksWait", { number: line.pull_request }) : null;
    case "children":
      return t("agents.line.children", { count: agent.descendant_counts?.working ?? 0 });
    case "background":
      return t("agents.line.background");
    case "reply":
      return t("agents.line.reply");
    default:
      return null;
  }
}

/** Whether the line is drawn without a pointer or selection on the row. */
export function lineShownAtRest(line: RowLine, selected: boolean): boolean {
  return line.mode !== "quiet" || selected;
}

/** The colour of a line: a request in its demand's colour, dimmed like its mark once read, news bright, failed checks in warning, a revealed or kept line subdued. */
export function lineTone(line: RowLine, agent: Pick<AgentRow, "state">): string {
  if (line.mode === "request") return toneClass(agent.state.chip_tone);
  if (line.mode === "checks_failed") return "text-warning";
  return line.mode === "news" || line.mode === "checks_passed" ? "text-foreground" : "text-subtle-foreground";
}

/** The status mark's dimming: a read turn draws at the read-status opacity. */
export function markTone(agent: Pick<TabAgent, "state">): string {
  return agent.state.mark_tone.read ? "opacity-(--opacity-read-status)" : "";
}

/** The name of a mark, in the operator's language: 내 차례, 진행 중, 읽을 것, 조용함, 재움. An unknown row has no mark to name. */
export function markName(t: TFunction<"translation">, mark: Exclude<RowMark, "unknown">): string {
  return t(`agents.mark.${mark}`);
}

/**
 * What a mark says on hover and in a row's accessible name (B3): the state,
 * then the word that leads its line, `내 차례 · 질문`, `읽을 것 · 일 남음`. A
 * quiet row adds the word only when it says something (일 남음, 질문); an in
 * progress row names nothing more, its line says why. A row of unknown state
 * has no mark and says only `상태 모름`. A sleeping row says its
 * own words, `재움 · 열면 깨어남` (B31), and a finished turn with tasks left
 * adds them, `읽을 것 · 끝남 · 백그라운드 2개 남음` (B27).
 */
export function markWords(t: TFunction<"translation">, agent: Pick<AgentRow, "mark" | "status_code"> & { background_left?: number }): string {
  if (agent.mark === "asleep") return statusText(t, agent.status_code);
  if (agent.mark === "unknown") return statusText(t, "unknown");
  const name = markName(t, agent.mark);
  const word = agent.mark === "going" ? null : agent.mark === "waiting" ? statusText(t, "waiting") : agent.mark === "quiet" ? (QUIET_WORDS.has(agent.status_code) ? statusText(t, agent.status_code) : null) : agent.mark === "unread" && agent.status_code === "stopped" ? statusText(t, agent.status_code) : leadWord(t, agent);
  const left = agent.background_left ? t("agents.backgroundLeftWords", { count: agent.background_left }) : null;
  return [name, word, left].filter(Boolean).join(" · ");
}

const QUIET_WORDS = new Set<AgentStatusCode>(["unfinished", "error", "question", "approval"]);

/** A row a surface draws dimmed with no mark: an agent the core marks unknown (B16). A disconnected device's rows are dimmed by their surface. */
export function rowDimmed(agent: Pick<AgentRow, "mark">): boolean {
  return agent.mark === "unknown";
}

/** A row the folded checkout still shows and the digits still number: it asks for the operator, and a resolved session rests however it ended. */
export function asksOperator(agent: Pick<AgentRow, "state" | "resolved">): boolean {
  return agent.state.needs_you && !agent.resolved;
}

/** A badge's counts: a project's or checkout's agents, or a parent's children. */
export type BadgeCounts = Partial<MarkCounts>;

/** One mark and count on a badge. */
export type BadgePart = { state: keyof MarkCounts; mark: RowMark; count: number };

const BADGE_ORDER: (keyof MarkCounts)[] = ["turn", "going", "done", "unread"];

/**
 * The badge's marks in the order whose move is next (B19): `!` N, ring N,
 * green dot N, plain dot N, zero states left out. A quiet agent, at rest, is
 * counted in none.
 */
export function badgeParts(counts: BadgeCounts | undefined): BadgePart[] {
  if (!counts) return [];
  return BADGE_ORDER.filter((state) => (counts[state] ?? 0) > 0).map((state) => ({ state, mark: state, count: counts[state] ?? 0 }));
}

/** A status badge in words, for the accessible name of the row it sits on: `내 차례 1개, 읽을 것 2개`. */
export function badgeWords(counts: BadgeCounts | undefined, t: TFunction<"translation">): string {
  return badgeParts(counts)
    .map((part) => t(`agents.count.${part.state}`, { count: part.count }))
    .join(", ");
}

/**
 * The branch chip: only a delegated row whose checkout differs from its
 * parent's carries one (B9). The core decides the difference and names the
 * checkout; a root's context in a group list has no checkout to differ from.
 */
export function branchChip(agent: Pick<AgentRow, "state">): string | null {
  return agent.state.branch_badge;
}

/**
 * The row's accessible name: title, the device and branch chips the row
 * shows, kind, status word, a letter waiting, then the sentence
 * (docs/status-model.md). The
 * row's visible text is hidden from assistive technology, so everything it
 * shows has to be here.
 */
export function rowAccessibleName(t: TFunction<"translation">, agent: AgentRow, device: string | null): string {
  return [agent.identity_label, recordWords(t,agent), agent.record_summary, device, branchChip(agent), agentProductName(agent.agent_kind), markWords(t, agent), hideAiWords(t, agent.hide_ai), agent.letters_waiting ? t("agents.letterWaiting") : null, rowSentence(t, agent)].filter(Boolean).join(", ");
}

export function recordPending(agent: Pick<AgentRow, "record_connection">): boolean {
  return agent.record_connection === "checking" || agent.record_connection === "unconfirmed";
}

export function recordWords(t: TFunction<"translation">, agent: Pick<AgentRow, "record_connection">): string | null {
  if (agent.record_connection === "checking") return t("agents.record.checking");
  if (agent.record_connection === "unconfirmed") return t("agents.record.unconfirmed");
  return null;
}

/**
 * The sentence a row's hover and accessible name end with: a waiting ring's
 * whole line, so `대기 · …` tells it from a working ring without sight
 * (B29), else the agent's own sentence.
 */
export function rowSentence(t: TFunction<"translation">, agent: AgentRow): string | null {
  const line = rowLine(agent);
  if (line?.mode === "waiting" || line?.subject) return lineText(t, agent, line);
  return agent.detail?.trim() || null;
}

/**
 * What a fork's mark says (PRD fork-operator-root B8, B9): the source by its title
 * while it still runs the conversation the fork came from, else that it came from
 * another pane. Null on a row that is not a fork.
 */
export function forkLabel(t: TFunction<"translation">, agent: Pick<AgentRow, "forked_from">): string | null {
  const source = agent.forked_from;
  if (!source) return null;
  return source.pane_id && source.label ? t("panes.fork.forkedFrom", { name: source.label }) : t("panes.fork.forked");
}

/** A row in a drawn agent tree: the row, its device, and how deep it sits under the root drawn above it. */
export type TreeRow = { agent: AgentRow; device: string | null; depth: number; descendants: number };
