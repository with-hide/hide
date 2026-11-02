// Chapter 0's world: one Claude agent the learner hands a job of several pull
// requests as a mission, so it conducts it (`hide-mission`). Whatever words
// the learner sends play the next part of the script, since a practice
// understands no words. The first words are the mission: the Conductor plans
// it, starts its workers in tabs of their own, one node finishes, and it asks
// one decision (`hide mission decide`) with its choices, which its row asks as
// any agent asks. A choice pressed on the map's banner, an answer written
// there, or the next words said to the Conductor answer it: the Conductor
// folds one node into another, clears the decision and goes on, and the
// Project's Memory keeps what was decided. The sidebar turns to its 태스크
// list and the lane shows the Memory tool as the core would on their events.
// Every word the world shows is in the learner's language
// (`practice.mission.world.*`), since this is the chapter a newcomer meets first.

import type { MessageKey } from "../../i18n/catalogs";
import { clientI18n } from "../../i18n/translator";
import { nextSidebarLens, type AgentRow, type AgentState, type DetailItem, type MemorySection, type Mission, type MissionNode, type PaneHeader, type SidebarLens } from "../../snapshot";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import type { ProjectTasksScope } from "../../agentScope";
import { agent, applyDesk, counted, deskSnapshot, hideAiOn, keyText, shellScreen, typed, withLineage, withTasks, type DeskModel, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };

/** The Conductor's pane, and the pane of the worker still at it. */
export const CONDUCTOR = "cd";
export const WORKER = "wk";
const WORKER_TAB = "t2";

const MISSION_ID = "practice-mission";

/** One of the world's words in the learner's language. */
const say = (key: string, values?: Record<string, string | number>) => clientI18n.t(`practice.mission.world.${key}` as MessageKey, values as never) as string;
const title = () => say("title");
const decision = () => say("decision");
/** The decision's choices, as `hide mission decide --option` gives them; the first is the one recommended. */
const options = () => [say("option1"), say("option2")];

/** 0 before the mission, 1 once it runs and a decision waits, 2 once the decision is answered and the plan changed. */
export type MissionStage = 0 | 1 | 2;

export type MissionModel = {
  desk: DeskModel;
  stage: MissionStage;
  /** What the learner has typed into the Conductor's prompt and not sent yet. */
  draft: string;
  /** Each thing the Conductor was told, in the learner's words. */
  said: string[];
  /** When the mission was made. */
  startedAt: number;
  /** The 보기 slot: what it opened, oldest first, and whether the lane draws it. */
  detail: DetailItem[];
  detailShown: boolean;
  /** The core's count of lane calls; an open in the 보기 slot is one, which brings an unpinned lane up. */
  laneCalls: number;
  /** Which list the sidebar shows (`sidebar_lens_set`). */
  lens: SidebarLens;
  /** Whether the lane shows the Project's Memory tool beside the 보기 slot. */
  memory: boolean;
};

/** Whether the sidebar shows its 태스크 list. */
export const showsTasks = (model: MissionModel) => model.lens === "tasks";

/** Whether the lane shows the Memory tool. */
export const showsMemory = (model: MissionModel) => model.memory;

/** Whether the 보기 slot shows this chapter's mission map in front. */
export function missionInSlot(model: MissionModel): boolean {
  const top = model.detail.at(-1);
  return model.detailShown && top?.kind === "mission" && top.mission === MISSION_ID;
}

// --- The rows --------------------------------------------------------------------

const tone = (kind: "warning" | "subtle", read: boolean) => ({ kind, read });

/** A root at rest, as the core shows a finished, read one: Seen. */
function resting(row: AgentRow): AgentRow {
  const state: AgentState = {
    ...row.state,
    working: false, chip_tone: tone("subtle", true), mark_tone: tone("subtle", true),
    bucket: "resting", attention_rank: 4, search_tone: "muted", subtree: "quiet", link: "idle", link_rank: 2, verb: "idle", tree_rank: 4,
    session: { group: "idle", line: null, unfinished: false },
  };
  return { ...row, state, group: "seen", mark: "quiet", status_code: "idle", activity: "idle", emphasized: false };
}

/** The Conductor asking its decision in conversation: an ordinary question, in Needs You (status-model.md, A mission adds no status). */
function asking(row: AgentRow, since: number): AgentRow {
  const state: AgentState = {
    ...row.state,
    attention: true, needs_you: true, title_emphasized: true, asking: true, working: false,
    chip_tone: tone("warning", false), mark_tone: tone("warning", false), bucket: "turn", attention_rank: 1, link_rank: 0, search_tone: "attention", subtree: "waiting", link: "question",
    verb: "answer", tree_rank: 0, request_since: since,
    session: { group: "needs_you", line: null, unfinished: false },
    ask: { verb: "answer", what: decision(), more: 0 },
    line: { text: decision(), mode: "request", tone: tone("warning", false) },
  };
  return {
    ...row, state, group: "needs_you", mark: "turn", status_code: "question", demand: "question", activity: "idle", emphasized: true, unread: true,
    user_turn: { kind: "question", content: null }, changed_at_unix_ms: since,
  };
}

const conductorRow = (spec: Pick<AgentRow, "group" | "mark" | "status_code"> & { elapsed: string }) =>
  agent({ pane_id: CONDUCTOR, identity_label: title(), agent_kind: "claude", activity: spec.status_code === "working" ? "working" : "idle", ...spec });

const COUNTS = (working: number) => ({ error: 0, approval: 0, question: 0, working, done: 0 });

/** The worker on the backoff node: delegated by the Conductor, in a tab of its own off the strip. */
function workerRow(): AgentRow {
  const row: AgentRow = {
    ...agent({ pane_id: WORKER, identity_label: say("node.backoff"), agent_kind: "codex", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "2m" }),
    delegated: true, lineage_parent_pane_id: CONDUCTOR, lineage_path_pane_ids: [CONDUCTOR], lineage_depth: 1,
  };
  row.state = { ...row.state, root: false };
  return row;
}

function initial(): MissionModel {
  const conductor = resting(conductorRow({ group: "seen", mark: "quiet", status_code: "idle", elapsed: "8m" }));
  const desk: DeskModel = {
    panes: [{ id: CONDUCTOR, agent: conductor, title: title() }],
    tabs: [{ id: "t1", name: null, root: { type: "pane", pane_id: CONDUCTOR }, focused: CONDUCTOR, zoomed: false }],
    groups: [{ id: null, name: null, tabs: ["t1"] }],
    selectedGroup: null,
    activeTab: "t1",
    // Past the worker's tab, so a tab or split the learner opens takes a new number.
    nextId: 3,
    operations: [],
  };
  return { desk, stage: 0, draft: "", said: [], startedAt: Date.now(), detail: [], detailShown: false, laneCalls: 0, lens: "projects", memory: false };
}

/** The desk with the Conductor's row changed by `change`. */
function withConductor(desk: DeskModel, change: (row: AgentRow) => AgentRow): DeskModel {
  return { ...desk, panes: desk.panes.map((pane) => (pane.id === CONDUCTOR && pane.agent ? { ...pane, agent: change(pane.agent) } : pane)) };
}

/** The mission started: the worker on the backoff node in its own tab, under the Conductor, and the Conductor asking. */
function started(desk: DeskModel, now: number): DeskModel {
  const worker = workerRow();
  const withWorker: DeskModel = {
    ...desk,
    panes: [...desk.panes, { id: WORKER, agent: worker, title: worker.identity_label }],
    tabs: [...desk.tabs, { id: WORKER_TAB, name: null, root: { type: "pane", pane_id: WORKER }, focused: WORKER, zoomed: false }],
    groups: desk.groups.map((group) => (group.id === null ? { ...group, tabs: [...group.tabs, WORKER_TAB] } : group)),
  };
  return withConductor(withWorker, (row) => ({
    ...asking(row, now),
    lineage_child_pane_ids: [WORKER], lineage_collapsed: false, direct_child_counts: COUNTS(1), descendant_counts: COUNTS(1),
  }));
}

// --- The mission ---------------------------------------------------------------------

const PR_URL = (number: number) => `https://example.com/${PLACE.project}/pull/${number}`;

const NODE: Omit<MissionNode, "key" | "title" | "status" | "worker"> = {
  kind: "change", depends_on: [], blocked: null, delivery: "pr", agent_id: null, pane_id: null, machine: null,
  pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 1,
  line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, landing: null, verdict: null, checks: null, plan: null, prs: [], attention: null, hide: null,
};

/** The plan's three nodes, as the record holds them at `stage`. */
function nodes(model: MissionModel): MissionNode[] {
  const minute = 60_000;
  const at = model.startedAt;
  const answered = model.stage >= 2;
  return [
    {
      ...NODE, key: "config", title: say("node.config"), status: "done",
      worker: { agent: "codex", model: "gpt-5.5", effort: "medium" },
      pr: 21, prs: [{ number: 21, state: "mergeable", url: PR_URL(21) }], pr_state: "mergeable", landing: { state: "left", action: "merge" }, pr_url: PR_URL(21), branch: "practice/retry-config", stage: 3,
      line: say("configLine"), review: say("configReview"),
      started_at_unix_ms: at - 9 * minute, finished_at_unix_ms: at - 2 * minute,
    },
    {
      ...NODE, key: "backoff", title: say("node.backoff"), status: "working",
      worker: { agent: "codex", model: "gpt-5.5", effort: "high", effort_auto: true },
      agent_id: "agent-2", pane_id: WORKER, branch: "practice/retry-backoff",
      ...(answered
        ? { pr: 22, prs: [{ number: 22, state: "pending" as const, url: PR_URL(22) }], pr_state: "pending" as const, pr_url: PR_URL(22), stage: 2, line: say("backoffLineAfter"), change: say("backoffChange") }
        : { stage: 1, line: say("backoffLine") }),
      started_at_unix_ms: at - 9 * minute,
    },
    {
      ...NODE, key: "docs", title: say("node.docs"), depends_on: ["backoff"],
      status: answered ? "dropped" : "planned",
      worker: { agent: "claude", model: "sonnet", effort: "low" },
      change: answered ? say("docsChange") : null,
    },
  ];
}

function mission(model: MissionModel): Mission | null {
  if (model.stage === 0) return null;
  const now = Date.now();
  return {
    id: MISSION_ID, title: title(), workspace_id: PLACE.project, project: PLACE.label, checkout_path: `/work/${PLACE.project}`, device_id: "local",
    status: "running", scope: "pr", cap: 2, exempt: false, strength: "normal", other_vendor: true,
    // No session: this practice keeps no record to open.
    conductor: { pane_id: CONDUCTOR, name: title(), agent_kind: "claude", session: null, present: true, takes_answers: true },
    nodes: nodes(model), merge_order: ["config", "backoff"], landing_order: ["config"], stack_waiting: 0, check: "pnpm test", github: "practice-bot",
    decision: model.stage === 1 ? decision() : null,
    question: model.stage === 1 ? { key: 1, why: say("why"), options: options(), recommend: 1, node: "docs", asked_at_unix_ms: model.startedAt } : null,
    answer: null, answer_failed: null, delay: null, report: null,
    created_at_unix_ms: model.startedAt - 10 * 60_000, started_at_unix_ms: model.startedAt - 9 * 60_000, closed_at_unix_ms: null,
    running: 1, stale: false, github_stale: false, github_read_at_unix_ms: now,
  };
}

// --- The Conductor's prompt ------------------------------------------------------------

/**
 * The decision answered with `words`: `hide mission decide --clear`, so the
 * row stops asking and the Conductor goes back to work on the changed plan.
 */
function answered(model: MissionModel, words: string): MissionModel {
  return {
    ...model, stage: 2, said: [...model.said, words],
    desk: withConductor(model.desk, (row) => ({
      ...conductorRow({ group: "working", mark: "going", status_code: "working", elapsed: "1s" }),
      lineage_child_pane_ids: row.lineage_child_pane_ids, lineage_collapsed: false, direct_child_counts: COUNTS(1), descendant_counts: COUNTS(1),
    })),
  };
}

/**
 * The prompt, sent: the first words are the mission, the next the decision's
 * answer; words after that the Conductor takes on as it goes. An empty prompt
 * sends nothing.
 */
function submit(model: MissionModel): MissionModel {
  const words = model.draft.trim();
  if (!words) return { ...model, draft: "" };
  const now = Date.now();
  switch (model.stage) {
    case 0:
      return { ...model, draft: "", said: [words], stage: 1, startedAt: now, desk: started(model.desk, now) };
    case 1:
      return answered({ ...model, draft: "" }, words);
    default:
      return { ...model, draft: "", said: [...model.said, words] };
  }
}

/**
 * The banner's answer, as the core types it into the Conductor's conversation:
 * the question, then the choice pressed or the words written. An answer to
 * another mission or question, or once nothing is asked, is refused.
 */
function answerLine(model: MissionModel, payload: Record<string, unknown>): string | null {
  if (model.stage !== 1 || payload.mission !== MISSION_ID || payload.key !== 1) return null;
  const answer = typeof payload.option === "number" ? options()[payload.option - 1] : typeof payload.text === "string" ? payload.text.trim() : undefined;
  return answer ? say("answerLine", { question: decision(), answer }) : null;
}

// --- The events --------------------------------------------------------------------------

function apply(model: MissionModel, event: CoreEvent): MissionModel | null {
  const p = event.payload;
  switch (event.kind) {
    // Only the Conductor is talked to here; keys typed into another pane reach no one.
    case "key":
      return p.pane_id === CONDUCTOR && typeof p.bytes_base64 === "string" ? typed(model, keyText(p.bytes_base64), submit) : null;
    case "mission_answer": {
      const line = answerLine(model, p);
      return line ? answered(model, line) : null;
    }
    // The map opens in the 보기 slot as a lane call, the way the core numbers one; a record or a card this world keeps none of.
    case "detail_open": {
      const item = p.item as DetailItem;
      if (item.kind !== "mission" || item.mission !== MISSION_ID || model.stage === 0) return null;
      return { ...model, detail: [...model.detail.filter((one) => one.kind !== "mission"), item], detailShown: true, laneCalls: model.laneCalls + 1 };
    }
    case "detail_back":
      return model.detail.length > 1 ? { ...model, detail: model.detail.slice(0, -1) } : null;
    case "detail_close":
      return model.detail.length ? { ...model, detail: [], detailShown: false } : null;
    case "detail_show":
      return model.detail.length ? { ...model, detailShown: p.shown === true } : null;
    case "sidebar_lens_set": {
      const lens = (p.lens as SidebarLens | undefined) ?? nextSidebarLens(model.lens);
      return lens === model.lens ? null : { ...model, lens };
    }
    // The Memory tool shown, by its rail icon, ⇧⌘L or from elsewhere; the lane put away or another tool shown takes it down.
    case "project_memory_navigate":
      return p.workspace_id === PLACE.project && !model.memory ? { ...model, memory: true } : null;
    case "workspace_view": {
      const memory = p.tool === "memory" ? true : p.open === false || "tool" in p ? false : model.memory;
      return memory === model.memory ? null : { ...model, memory };
    }
    default: {
      const desk = applyDesk(model.desk, event);
      return desk ? { ...model, desk } : null;
    }
  }
}

// --- The snapshot ------------------------------------------------------------------------

/** The Conductor pane's head as the core sends it: at rest, asking on its second row, or at work on the answer. */
function conductorHeader(model: MissionModel): PaneHeader {
  if (model.stage === 1) return { working: false, notice: null, band: { kind: "answer", reason: decision(), since_unix_ms: model.startedAt, read: false, action: null, more: 0 } };
  if (model.stage === 2) return { working: true, notice: null, band: null, request: { text: model.said.at(-1)!, images: 0 } };
  return { working: false, notice: null, band: null };
}

function snapshot(model: MissionModel, nowMs: number): WorldSnapshot {
  const drawn = deskSnapshot([{ place: PLACE, model: model.desk }], PLACE.project, nowMs);
  // Once the mission runs, its worker is drawn under the Conductor; the Conductor rests before it and asks while the decision waits.
  const lineage = model.stage >= 1 ? withLineage(drawn.rest, CONDUCTOR, [WORKER]) : drawn.rest;
  const base = withTasks(model.stage === 2 ? lineage : counted(lineage, model.stage === 1 ? "needs_you" : "seen"), tasks(model, nowMs));
  const shown = model.detailShown && model.detail.length > 0;
  const current = mission(model);
  const headers: Record<string, PaneHeader> = { [CONDUCTOR]: conductorHeader(model) };
  if (model.stage >= 1) headers[WORKER] = { working: true, notice: null, band: null };
  return {
    ...drawn,
    rest: {
      ...base,
      missions: { items: current ? [current] : [] },
      ui_state: { ...base.ui_state!, sidebar_lens: model.lens, detail: { items: model.detail, shown } },
      // Opening the 보기 slot opens the lane, as the core does on `detail_open`; the Memory tool stands beside it.
      workspace_view: { ...base.workspace_view!, open: shown || model.memory, tool: model.memory ? "memory" : null, lane_called: model.laneCalls, lane_calls: model.laneCalls, lane_called_side: "view" },
      terminal: { headers },
      // Hide AI on, so the Memory tool reads as it does for an operator who keeps one.
      status: { ...base.status, background_ai: { enabled: true, provider: "claude", chosen: true, agent_summary: true, providers: [], unavailable_reason: null } },
    },
    frames: { memory: memory(model, nowMs), suggestions: hideAiOn(PLACE.project) },
  };
}

/** The 태스크 list's one task: the Conductor with its worker, in 내 차례 while the decision waits. */
function tasks(model: MissionModel, nowMs: number): ProjectTasksScope {
  const state = model.stage === 1 ? "needs_you" as const : model.stage === 2 ? "working" as const : "resting" as const;
  const members = model.stage >= 1 ? [CONDUCTOR, WORKER] : [CONDUCTOR];
  const recency = new Date(nowMs).toISOString();
  return { cards: [{ root: CONDUCTOR, state, members, checkouts: [`${PLACE.project}:${PLACE.branch}`], recency, stage: 1 }], needs_you: model.stage === 1 ? 1 : 0, recency };
}

// --- The Project's Memory ------------------------------------------------------------------

const DAY_MS = 24 * 60 * 60_000;

/**
 * What the Project's Memory keeps: two older memories, and once the decision is
 * answered the one Hide AI made of it, newest first, from the operator's own words.
 */
function memory(model: MissionModel, nowMs: number): MemorySection {
  type Card = [kind: "decision" | "rule" | "lesson", key: string, ageMs: number, mine: number];
  const decided: Card[] = model.stage >= 2 ? [["decision", "memoryNew", 60_000, 1]] : [];
  const cards: Card[] = [...decided, ["lesson", "memoryLesson", 3 * DAY_MS, 0], ["rule", "memoryRule", 6 * DAY_MS, 1]];
  return {
    project: {
      workspace_id: PLACE.project, unavailable: null, loading: false, enabled: true, capacity_reached: false, evicted_cards: 0, evicted_through_unix_ms: null,
      rows: cards.map(([kind, key, ageMs, mine], index) => ({
        number: cards.length - index, title: say(key), kind, human_source_count: mine, provided_session_count: index, pull_count: index, updated_at_unix_ms: nowMs - ageMs, verifying: false, promotion: null,
      })),
      questions: [], analysis: "idle", consent_required: false, usable_cards: cards.length, checking_card: null, check_resume_at_unix_ms: null,
      forgotten: null, pending_action: null, expanded: null, search: null,
    },
  };
}

// --- What the terminals show -----------------------------------------------------------------

const CLAUDE = "\x1b[38;2;215;119;87m";
const CODEX = "\x1b[36m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const GREEN = "\x1b[32m";
const YELLOW = "\x1b[33m";
const RESET = "\x1b[0m";
const rule = (cols: number) => `${DIM}${"─".repeat(cols)}${RESET}`;

/** The Conductor's conversation: the mission it took, its plan and its workers, the decision it asks, and the plan it changed on the answer. */
function conductorScreen(model: MissionModel, cols: number): string {
  const said = (words: string) => ["", `${DIM}>${RESET} ${words}`, ""];
  const ran = (command: string, result: string) => [`● ${BOLD}Bash${RESET}(${command})`, `  ${DIM}⎿${RESET}  ${result}`];
  const rows = [
    "",
    "",
    `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`,
    `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`,
    ...said(say("warmupAsk")),
    `● ${say("warmupAnswer")}`,
  ];
  if (model.stage >= 1) {
    rows.push(
      ...said(model.said[0]!),
      ...ran(`hide mission create --title "${title()}"`, say("made")),
      ...ran("hide mission plan --file plan.json", say("planned")),
      `● ${say("onChip")}`,
      ...ran("hide agent spawn --parent here …", say("started")),
      ...ran("hide mission node config --pr 21 --status done", `${GREEN}#21${RESET} ${say("node.config")} · ${say("ciPassed")}`),
      "",
      `● ${YELLOW}${say("decideHead")}${RESET} ${say("decideWhy")}`,
      `  ${decision()}`,
      ...ran("hide mission decide --option … --recommend 1", say("asked")),
    );
  }
  if (model.stage >= 2) {
    rows.push(
      ...said(model.said[1]!),
      ...ran("hide mission plan --file plan.json", say("folded")),
      ...ran("hide mission decide --clear", say("cleared")),
      `● ${say("doneLine")}`,
    );
    for (const later of model.said.slice(2)) rows.push(...said(later), `● ${say("noted")}`);
    rows.push("", `${CLAUDE}✻ ${say("waiting")} ${DIM}(esc to interrupt)${RESET}`);
  }
  rows.push("", rule(cols), `${DIM}>${RESET} ${model.draft}`, rule(cols));
  return rows.join("\r\n");
}

/** The backoff worker: the brief it started on, and the docs it took in once the plan changed. */
function workerScreen(model: MissionModel): string {
  const rows = [
    `${DIM}╭──────────────────────────────╮${RESET}`,
    `${DIM}│${RESET} ${BOLD}>_ Codex${RESET}                     ${DIM}│${RESET}`,
    `${DIM}│${RESET} ${DIM}~/${PLACE.project}-backoff${RESET}           ${DIM}│${RESET}`,
    `${DIM}╰──────────────────────────────╯${RESET}`,
    "",
    `${CODEX}›${RESET} ${say("brief", { title: title() })}`,
    "",
    `${DIM}•${RESET} ${BOLD}Edited${RESET} src/payment/retry.ts`,
  ];
  if (model.stage >= 2) rows.push("", `${CODEX}›${RESET} ${YELLOW}🦉  ${say("letterFrom", { title: title() })}${RESET}`, `  ${say("letterBody")}`, "", `${DIM}•${RESET} ${BOLD}Opened${RESET} #22 · ${say("opened")}`);
  return rows.join("\r\n");
}

export const MISSION_WORLD: PracticeWorld<MissionModel> = {
  initial,
  apply,
  snapshot,
  prompt: (model, paneId) => (paneId === CONDUCTOR ? model.draft : ""),
  panes: (model) => model.desk.panes.map((pane) => pane.id),
  // A pane the learner opened, a new tab or a split, is a shell with no agent in it.
  screen: (model, paneId, cols) => (paneId === CONDUCTOR ? conductorScreen(model, cols) : paneId === WORKER ? workerScreen(model) : shellScreen(PLACE)),
  // The mission's nodes stand for pull requests, which could pass for real ones; the mark sits on the title, clear of the card's marks.
  examples: ["[data-mission-node] > span:first-child > span[title]"],
};
