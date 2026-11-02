// Chapter 0's world: one job carried from the learner's request to the report,
// as a mission (`hide-mission`). Whatever words the learner sends play the next
// part of the script, since a practice understands no words. The first words
// are the mission: the Conductor plans it and asks whether to start. The next
// words approve it: the Conductor starts its worker in a tab of its own, one
// part is already a pull request, and it asks one decision (`hide mission
// decide`) with its choices. A choice pressed on the map's banner, an answer
// written there, or the next words said to the Conductor answer it: the
// Conductor folds one part into another and goes on. Once the learner has
// looked in on the worker and comes back to the Conductor, the worker is done,
// the Conductor reports, and the Project's Memory keeps what was decided.
// The sidebar turns to its 태스크 list, the lane shows the Memory tool and the
// Overview chooses an agent as the core would on their events.
// Every word the world shows is in the learner's language
// (`practice.mission.world.*`), since this is the chapter a newcomer meets first.

import type { ProjectTasksScope } from "../../agentScope";
import type { MessageKey } from "../../i18n/catalogs";
import { clientI18n } from "../../i18n/translator";
import { nextSidebarLens, type AgentChip, type AgentRow, type AgentState, type DetailItem, type MemorySection, type Mission, type MissionNode, type PaneFocusRequest, type PaneHeader, type SidebarLens } from "../../snapshot";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, counted, deskSnapshot, focusedPane, hideAiOn, keyText, shellScreen, typed, withLineage, withTasks, type DeskModel, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };

/** The Conductor's pane, and the panes of its two workers: the backoff part's, which the learner looks in on, and the settings part's. */
export const CONDUCTOR = "cd";
export const WORKER = "wk";
export const CONFIG_WORKER = "wc";
const WORKER_TAB = "t2";
const CONFIG_TAB = "t3";

const MISSION_ID = "practice-mission";

/** One of the world's words in the learner's language. */
const say = (key: string, values?: Record<string, string | number>) => clientI18n.t(`practice.mission.world.${key}` as MessageKey, values as never) as string;
const title = () => say("title");
const decision = () => say("decision");
/** The decision's choices, as `hide mission decide --option` gives them; the first is the one recommended. */
const options = () => [say("option1"), say("option2")];

/**
 * Where the mission stands: 0 before it, 1 planned and waiting for the
 * learner's go, 2 running with a decision waiting, 3 answered and at work,
 * 4 done and reported.
 */
export type MissionStage = 0 | 1 | 2 | 3 | 4;

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
  /** Whether the learner looked in on the worker after answering; coming back to the Conductor then finishes the mission. */
  visited: boolean;
  /** The core's receipt for the last focus asked by relationship (the tree's child, the head's parent mark). */
  focus: PaneFocusRequest | null;
};

/** Whether the sidebar shows its 태스크 list. */
export const showsTasks = (model: MissionModel) => model.lens === "tasks";

/** Whether the lane shows the Memory tool. */
export const showsMemory = (model: MissionModel) => model.memory;

/** The pane holding the keyboard. */
export const focusedOn = (model: MissionModel) => focusedPane(model.desk);

/** Whether the 보기 slot shows this chapter's mission map in front. */
export function missionInSlot(model: MissionModel): boolean {
  const top = model.detail.at(-1);
  return model.detailShown && top?.kind === "mission" && top.mission === MISSION_ID;
}

// --- The rows --------------------------------------------------------------------

const tone = (kind: "warning" | "subtle", read: boolean) => ({ kind, read });

/** An agent at rest, as the core shows a finished, read one: Seen. */
function resting(row: AgentRow): AgentRow {
  const state: AgentState = {
    ...row.state,
    working: false, chip_tone: tone("subtle", true), mark_tone: tone("subtle", true),
    bucket: "resting", attention_rank: 4, search_tone: "muted", subtree: "quiet", link: "idle", link_rank: 2, verb: "idle", tree_rank: 4,
    session: { group: "idle", line: null, unfinished: false },
  };
  return { ...row, state, group: "seen", mark: "quiet", status_code: "idle", activity: "idle", emphasized: false };
}

/** The Conductor asking `what` in conversation: an ordinary question, in Needs You (status-model.md, A mission adds no status). */
function asking(row: AgentRow, what: string, since: number): AgentRow {
  const state: AgentState = {
    ...row.state,
    attention: true, needs_you: true, title_emphasized: true, asking: true, working: false,
    chip_tone: tone("warning", false), mark_tone: tone("warning", false), bucket: "turn", attention_rank: 1, link_rank: 0, search_tone: "attention", subtree: "waiting", link: "question",
    verb: "answer", tree_rank: 0, request_since: since,
    session: { group: "needs_you", line: null, unfinished: false },
    ask: { verb: "answer", what, more: 0 },
    line: { text: what, mode: "request", tone: tone("warning", false) },
  };
  return {
    ...row, state, group: "needs_you", mark: "turn", status_code: "question", demand: "question", activity: "idle", emphasized: true, unread: true,
    user_turn: { kind: "question", content: null }, changed_at_unix_ms: since,
  };
}

const COUNTS = (working: number) => ({ error: 0, approval: 0, question: 0, working, done: 0 });

/** The workers at work at the stage: both once the mission runs, the backoff one alone once the settings part is a PR, none once it is done. */
function busyWorkers(stage: MissionStage): number {
  return stage === 2 ? 2 : stage === 3 ? 1 : 0;
}

/** The Conductor as the stage has it: at rest before and after, asking for its go and its decision, at work in between. */
function conductorRow(model: MissionModel): AgentRow {
  const working = model.stage === 3;
  const base = agent({
    pane_id: CONDUCTOR, identity_label: title(), agent_kind: "claude", session_id: "practice-conductor",
    group: working ? "working" : "seen", mark: working ? "going" : "quiet", status_code: working ? "working" : "idle", activity: working ? "working" : "idle", elapsed: working ? "1s" : "8m",
  });
  const row = model.stage === 1 ? asking(base, say("approveQuestion"), model.startedAt) : model.stage === 2 ? asking(base, decision(), model.startedAt) : working ? base : resting(base);
  if (model.stage < 2) return row;
  const busy = busyWorkers(model.stage);
  return { ...row, lineage_child_pane_ids: [CONFIG_WORKER, WORKER], lineage_collapsed: false, direct_child_counts: COUNTS(busy), descendant_counts: COUNTS(busy) };
}

/** A worker the Conductor delegated a part to, at work until `rests` says its part is done. */
function workerRow(paneId: string, part: "config" | "backoff", rests: boolean): AgentRow {
  const base: AgentRow = {
    ...agent({ pane_id: paneId, identity_label: say(`node.${part}`), agent_kind: "codex", session_id: `practice-worker-${part}`, group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "2m" }),
    delegated: true, lineage_parent_pane_id: CONDUCTOR, lineage_path_pane_ids: [CONDUCTOR], lineage_depth: 1,
  };
  const row = rests ? resting(base) : base;
  return { ...row, state: { ...row.state, root: false } };
}

/** A child as its parent's lineage lists it. */
function chip(row: AgentRow): AgentChip {
  return {
    tone: row.state.chip_tone, pane_id: row.pane_id, label: row.identity_label, detail: null, status_word_visible: false,
    agent_kind: row.agent_kind, demand: row.demand ?? "none", activity: row.activity ?? "idle", emphasized: row.emphasized, mark: row.mark, status_code: row.status_code, delegated: true,
  };
}

function initial(): MissionModel {
  const desk: DeskModel = {
    panes: [{ id: CONDUCTOR, agent: null, title: title() }],
    tabs: [{ id: "t1", name: null, root: { type: "pane", pane_id: CONDUCTOR }, focused: CONDUCTOR, zoomed: false }],
    groups: [{ id: null, name: null, tabs: ["t1"] }],
    selectedGroup: null,
    activeTab: "t1",
    // Past the workers' tabs, so a tab or split the learner opens takes a new number.
    nextId: 4,
    operations: [],
  };
  return { desk, stage: 0, draft: "", said: [], startedAt: Date.now(), detail: [], detailShown: false, laneCalls: 0, lens: "projects", memory: false, visited: false, focus: null };
}

/** The mission started: each worker in its own tab. */
function started(desk: DeskModel): DeskModel {
  return {
    ...desk,
    panes: [...desk.panes, { id: WORKER, agent: null, title: say("node.backoff") }, { id: CONFIG_WORKER, agent: null, title: say("node.config") }],
    tabs: [
      ...desk.tabs,
      { id: WORKER_TAB, name: null, root: { type: "pane", pane_id: WORKER }, focused: WORKER, zoomed: false },
      { id: CONFIG_TAB, name: null, root: { type: "pane", pane_id: CONFIG_WORKER }, focused: CONFIG_WORKER, zoomed: false },
    ],
    groups: desk.groups.map((group) => (group.id === null ? { ...group, tabs: [...group.tabs, WORKER_TAB, CONFIG_TAB] } : group)),
  };
}

/** The desk with each pane's agent as the stage has it. */
function withAgents(model: MissionModel): DeskModel {
  const workers: Record<string, AgentRow> = model.stage >= 2 ? { [WORKER]: workerRow(WORKER, "backoff", model.stage === 4), [CONFIG_WORKER]: workerRow(CONFIG_WORKER, "config", model.stage >= 3) } : {};
  const rows: Record<string, AgentRow> = { [CONDUCTOR]: conductorRow(model), ...workers };
  return { ...model.desk, panes: model.desk.panes.map((pane) => ({ ...pane, agent: rows[pane.id] ?? pane.agent })) };
}

// --- The mission ---------------------------------------------------------------------

const PR_URL = (number: number) => `https://example.com/${PLACE.project}/pull/${number}`;
const MINUTE = 60_000;

const NODE: Omit<MissionNode, "key" | "title" | "status" | "worker"> = {
  kind: "change", depends_on: [], blocked: null, delivery: "pr", agent_id: null, pane_id: null, machine: null,
  pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 1,
  line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, landing: null, verdict: null, checks: null, plan: null, prs: [], attention: null, hide: null,
};

/** The plan's three parts, as the record holds them at the model's stage. */
function nodes(model: MissionModel): MissionNode[] {
  const at = model.startedAt;
  const planned = model.stage === 1;
  const answered = model.stage >= 3;
  const done = model.stage === 4;
  const configWorker = { agent: "codex" as const, model: "gpt-5.5", effort: "medium" as const };
  const config: MissionNode = planned
    ? { ...NODE, key: "config", title: say("node.config"), status: "planned", worker: configWorker }
    : !answered
      ? { ...NODE, key: "config", title: say("node.config"), status: "working", worker: configWorker, agent_id: "agent-1", pane_id: CONFIG_WORKER, branch: "practice/retry-config", stage: 1, line: say("configWorking"), started_at_unix_ms: at - 9 * MINUTE }
      : {
      ...NODE, key: "config", title: say("node.config"), status: "done",
      worker: configWorker,
      pr: 21, prs: [{ number: 21, state: "mergeable", url: PR_URL(21) }], pr_state: "mergeable", landing: { state: "left", action: "merge" }, pr_url: PR_URL(21), branch: "practice/retry-config", stage: 3,
      line: say("configLine"), review: say("configReview"), started_at_unix_ms: at - 9 * MINUTE, finished_at_unix_ms: at - 2 * MINUTE,
    };
  const backoffWorker = { agent: "codex" as const, model: "gpt-5.5", effort: "high" as const, effort_auto: true };
  const backoff: MissionNode = planned
    ? { ...NODE, key: "backoff", title: say("node.backoff"), status: "planned", worker: backoffWorker }
    : done
      ? {
        ...NODE, key: "backoff", title: say("node.backoff"), status: "done", worker: backoffWorker, branch: "practice/retry-backoff",
        pr: 22, prs: [{ number: 22, state: "mergeable", url: PR_URL(22) }], pr_state: "mergeable", landing: { state: "left", action: "merge" }, pr_url: PR_URL(22), stage: 3,
        line: say("configLine"), change: say("backoffChange"), started_at_unix_ms: at - 9 * MINUTE, finished_at_unix_ms: at,
      }
      : {
        ...NODE, key: "backoff", title: say("node.backoff"), status: "working", worker: backoffWorker, agent_id: "agent-2", pane_id: WORKER, branch: "practice/retry-backoff",
        ...(answered
          ? { pr: 22, prs: [{ number: 22, state: "pending" as const, url: PR_URL(22) }], pr_state: "pending" as const, pr_url: PR_URL(22), stage: 2, line: say("backoffLineAfter"), change: say("backoffChange") }
          : { stage: 1, line: say("backoffLine") }),
        started_at_unix_ms: at - 9 * MINUTE,
      };
  const docs: MissionNode = {
    ...NODE, key: "docs", title: say("node.docs"), depends_on: ["backoff"],
    status: answered ? "dropped" : "planned",
    worker: { agent: "claude", model: "sonnet", effort: "low" },
    change: answered ? say("docsChange") : null,
  };
  return [config, backoff, docs];
}

function mission(model: MissionModel): Mission | null {
  if (model.stage === 0) return null;
  const now = Date.now();
  const status = model.stage === 1 ? "planning" : model.stage === 4 ? "merge_wait" : "running";
  return {
    id: MISSION_ID, title: title(), workspace_id: PLACE.project, project: PLACE.label, checkout_path: `/work/${PLACE.project}`, device_id: "local",
    status, scope: "pr", cap: 2, exempt: false, strength: "normal", other_vendor: true,
    // No session: this practice keeps no record to open.
    conductor: { pane_id: CONDUCTOR, name: title(), agent_kind: "claude", session: null, present: true, takes_answers: true },
    nodes: nodes(model), merge_order: ["config", "backoff"], landing_order: model.stage >= 3 ? ["config"] : [], stack_waiting: 0, check: "pnpm test", github: "practice-bot",
    decision: model.stage === 2 ? decision() : null,
    question: model.stage === 2 ? { key: 1, why: say("why"), options: options(), recommend: 1, node: "docs", asked_at_unix_ms: model.startedAt } : null,
    answer: null, answer_failed: null, delay: null, report: model.stage === 4 ? say("report") : null,
    created_at_unix_ms: model.startedAt - 10 * MINUTE, started_at_unix_ms: model.stage >= 2 ? model.startedAt - 9 * MINUTE : null, closed_at_unix_ms: null,
    running: busyWorkers(model.stage), stale: false, github_stale: false, github_read_at_unix_ms: now,
  };
}

// --- The Conductor's prompt ------------------------------------------------------------

/** The decision answered with `words`: the Conductor folds the docs part into the backoff PR and goes back to work. */
function answered(model: MissionModel, words: string): MissionModel {
  return { ...model, stage: 3, said: [...model.said, words] };
}

/**
 * The prompt, sent: the first words are the mission, the next the go, the
 * next the decision's answer; words after that the Conductor takes on as it
 * goes. An empty prompt sends nothing.
 */
function submit(model: MissionModel): MissionModel {
  const words = model.draft.trim();
  if (!words) return { ...model, draft: "" };
  const sent = { ...model, draft: "" };
  switch (model.stage) {
    case 0:
      return { ...sent, said: [words], stage: 1, startedAt: Date.now() };
    case 1:
      return { ...sent, said: [...model.said, words], stage: 2, startedAt: Date.now(), desk: started(model.desk) };
    case 2:
      return answered(sent, words);
    default:
      return { ...sent, said: [...model.said, words] };
  }
}

/**
 * The banner's answer, as the core types it into the Conductor's conversation:
 * the question, then the choice pressed or the words written. An answer to
 * another mission or question, or once nothing is asked, is refused.
 */
function answerLine(model: MissionModel, payload: Record<string, unknown>): string | null {
  if (model.stage !== 2 || payload.mission !== MISSION_ID || payload.key !== 1) return null;
  const answer = typeof payload.option === "number" ? options()[payload.option - 1] : typeof payload.text === "string" ? payload.text.trim() : undefined;
  return answer ? say("answerLine", { question: decision(), answer }) : null;
}

// --- The events --------------------------------------------------------------------------

/** A focus moved, by a click, a key or a relationship: looking in on the worker after the answer, then coming back, finishes the mission. */
function focused(model: MissionModel, event: CoreEvent): MissionModel | null {
  const desk = applyDesk(model.desk, event);
  if (!desk) return null;
  const paneId = String(event.payload.pane_id);
  const requestId = event.payload.request_id;
  const focus: PaneFocusRequest | null = typeof requestId === "string" ? { request_id: requestId, target_pane_id: paneId, phase: "succeeded", message: null, retryable: false } : model.focus;
  const visited = model.visited || (model.stage === 3 && paneId === WORKER);
  const finished = model.stage === 3 && visited && paneId === CONDUCTOR;
  return { ...model, desk, focus, visited, stage: finished ? 4 : model.stage };
}

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
    case "focus_pane":
      return focused(model, event);
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
    // The Memory tool shown, by ⇧⌘L, its rail icon or from elsewhere; the lane put away or another tool shown takes it down.
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

/** The Conductor pane's head as the core sends it: asking for its go or its decision, at work on the answer, or at rest. */
function conductorHeader(model: MissionModel): PaneHeader {
  const band = (reason: string) => ({ kind: "answer" as const, reason, since_unix_ms: model.startedAt, read: false, action: null, more: 0 });
  if (model.stage === 1) return { working: false, notice: null, band: band(say("approveQuestion")) };
  if (model.stage === 2) return { working: false, notice: null, band: band(decision()) };
  if (model.stage === 3) return { working: true, notice: null, band: null, request: { text: model.said.at(-1)!, images: 0 } };
  return { working: false, notice: null, band: null };
}

/** The 태스크 list's one task: the Conductor with its worker, in 내 차례 while it asks. */
function tasks(model: MissionModel, nowMs: number): ProjectTasksScope {
  const state = model.stage === 1 || model.stage === 2 ? "needs_you" as const : model.stage === 3 ? "working" as const : "resting" as const;
  const members = model.stage >= 2 ? [CONDUCTOR, CONFIG_WORKER, WORKER] : [CONDUCTOR];
  const recency = new Date(nowMs).toISOString();
  return { cards: [{ root: CONDUCTOR, state, members, checkouts: [`${PLACE.project}:${PLACE.branch}`], recency, stage: 1 }], needs_you: state === "needs_you" ? 1 : 0, recency };
}

function snapshot(model: MissionModel, nowMs: number): WorldSnapshot {
  const desk = withAgents(model);
  const drawn = deskSnapshot([{ place: PLACE, model: desk }], PLACE.project, nowMs);
  // Once the mission runs, its workers are drawn under the Conductor, and every agent not at work is counted where it stands.
  const lineage = model.stage >= 2 ? withLineage(drawn.rest, CONDUCTOR, [CONFIG_WORKER, WORKER]) : drawn.rest;
  const asks = model.stage === 1 || model.stage === 2;
  const counts = asks ? counted(lineage, "needs_you") : counted(lineage, "seen", model.stage === 4 ? 3 : 1);
  const base = withTasks(counts, tasks(model, nowMs));
  const navigator = base.navigator!;
  // A worker's head carries the way back to its parent, as the core lists a child's lineage.
  const conductor = desk.panes.find((pane) => pane.id === CONDUCTOR)!.agent!;
  const workers = desk.panes.flatMap((pane) => (pane.agent?.lineage_parent_pane_id === CONDUCTOR ? [pane.agent] : []));
  const lineagePath = (paneId: string) => {
    const worker = workers.find((row) => row.pane_id === paneId);
    return worker ? [{ pane_id: CONDUCTOR, label: conductor.identity_label, siblings: [] }, { pane_id: paneId, label: worker.identity_label, siblings: workers.map(chip) }] : undefined;
  };
  const workspaces = navigator.workspaces!.map((workspace) => ({
    ...workspace,
    checkouts: workspace.checkouts.map((checkout) => ({ ...checkout, tabs: checkout.tabs.map((row) => ({ ...row, panes: row.panes.map((one) => ({ ...one, lineage_path: lineagePath(one.id) })) })) })),
  }));
  const shown = model.detailShown && model.detail.length > 0;
  const current = mission(model);
  const headers: Record<string, PaneHeader> = { [CONDUCTOR]: conductorHeader(model) };
  for (const worker of workers) headers[worker.pane_id] = { working: worker.state.working, notice: null, band: null };
  const agents = desk.panes.flatMap((pane) => (pane.agent ? [pane.agent] : []));
  return {
    ...drawn,
    rest: {
      ...base,
      navigator: { ...navigator, workspaces },
      missions: { items: current ? [current] : [] },
      ui_state: { ...base.ui_state!, sidebar_lens: model.lens, detail: { items: model.detail, shown } },
      // Opening the 보기 slot opens the lane, as the core does on `detail_open`; the Memory tool stands beside it.
      workspace_view: { ...base.workspace_view!, open: shown || model.memory, tool: model.memory ? "memory" : null, lane_called: model.laneCalls, lane_calls: model.laneCalls, lane_called_side: "view" },
      terminal: { headers },
      // Hide AI on, so the Memory tool reads as it does for an operator who keeps one; the relationship focus answered.
      status: { ...base.status, pane_focus_request: model.focus, background_ai: { enabled: true, provider: "claude", chosen: true, agent_summary: true, providers: [], unavailable_reason: null } },
      // The core's proof of each agent's session, which a pane's 개요에서 보기 needs to choose it on the map.
      overview_graph: {
        available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 30 * 24 * 60 * MINUTE, letters: [],
        sessions: agents.flatMap((row) => (row.session_id ? [{ device_id: "local", pane_id: row.pane_id, session_digest: row.session_id, launch_model: null, launch_effort: null }] : [])),
      },
    },
    frames: { memory: memory(model, nowMs), suggestions: hideAiOn(PLACE.project) },
  };
}

// --- The Project's Memory ------------------------------------------------------------------

const DAY_MS = 24 * 60 * MINUTE;

/**
 * What the Project's Memory keeps: two older memories, and once the mission is
 * done the one Hide AI made of the decision, newest first, from the learner's own words.
 */
function memory(model: MissionModel, nowMs: number): MemorySection {
  type Card = [kind: "decision" | "rule" | "lesson", key: string, ageMs: number, mine: number];
  const decided: Card[] = model.stage === 4 ? [["decision", "memoryNew", MINUTE, 1]] : [];
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

/** The Conductor's conversation: the mission it took and its plan, the go, its worker and the decision, the changed plan, and the report. */
function conductorScreen(model: MissionModel, cols: number): string {
  const said = (words: string) => ["", `${DIM}>${RESET} ${words}`, ""];
  const ran = (command: string, result: string) => [`● ${BOLD}Bash${RESET}(${command})`, `  ${DIM}⎿${RESET}  ${result}`];
  const rows = ["", "", `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`];
  if (model.stage >= 1) {
    rows.push(
      ...said(model.said[0]!),
      ...ran(`hide mission create --title "${title()}"`, say("made")),
      ...ran("hide mission plan --file plan.json", say("planned")),
      `● ${YELLOW}${say("planReady")}${RESET}`,
    );
  }
  if (model.stage >= 2) {
    rows.push(
      ...said(model.said[1]!),
      `● ${say("starting")}`,
      ...ran("hide agent spawn --parent here …", say("started")),
      "",
      `● ${YELLOW}${say("decideHead")}${RESET} ${say("decideWhy")}`,
      `  ${decision()}`,
      ...ran("hide mission decide --option … --recommend 1", say("asked")),
    );
  }
  if (model.stage >= 3) {
    rows.push(
      ...said(model.said[2]!),
      ...ran("hide mission node config --pr 21 --status done", `${GREEN}#21${RESET} ${say("node.config")} · ${say("ciPassed")}`),
      ...ran("hide mission plan --file plan.json", say("folded")),
      ...ran("hide mission decide --clear", say("cleared")),
      `● ${say("doneLine")}`,
    );
    for (const later of model.said.slice(3)) rows.push(...said(later), `● ${say("noted")}`);
  }
  if (model.stage === 3) rows.push("", `${CLAUDE}✻ ${say("waiting")} ${DIM}(esc to interrupt)${RESET}`);
  if (model.stage === 4) rows.push("", ...ran("hide mission report …", say("reported")), `● ${GREEN}${say("report")}${RESET}`);
  rows.push("", rule(cols), `${DIM}>${RESET} ${model.draft}`, rule(cols));
  return rows.join("\r\n");
}

/** The backoff worker: the brief it started on, the docs it took in once the plan changed, and its PR ready. */
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
  if (model.stage >= 3) rows.push("", `${CODEX}›${RESET} ${YELLOW}🦉  ${say("letterFrom", { title: title() })}${RESET}`, `  ${say("letterBody")}`, "", `${DIM}•${RESET} ${BOLD}Opened${RESET} #22 · ${say("opened")}`);
  if (model.stage === 3) rows.push("", `${DIM}•${RESET} ${say("workerTesting")}`);
  if (model.stage === 4) rows.push("", `${GREEN}✓${RESET} #22 · ${say("workerDone")}`);
  return rows.join("\r\n");
}

/** The settings worker: its brief, and its PR once it is done. */
function configScreen(model: MissionModel): string {
  const rows = [
    `${DIM}╭──────────────────────────────╮${RESET}`,
    `${DIM}│${RESET} ${BOLD}>_ Codex${RESET}                     ${DIM}│${RESET}`,
    `${DIM}│${RESET} ${DIM}~/${PLACE.project}-config${RESET}            ${DIM}│${RESET}`,
    `${DIM}╰──────────────────────────────╯${RESET}`,
    "",
    `${CODEX}›${RESET} ${say("briefConfig", { title: title() })}`,
    "",
    `${DIM}•${RESET} ${BOLD}Edited${RESET} src/payment/config.ts`,
  ];
  rows.push("", model.stage >= 3 ? `${GREEN}✓${RESET} #21 · ${say("ciPassed")}` : `${DIM}•${RESET} ${say("workerTesting")}`);
  return rows.join("\r\n");
}

export const MISSION_WORLD: PracticeWorld<MissionModel> = {
  initial,
  apply,
  snapshot,
  prompt: (model, paneId) => (paneId === CONDUCTOR ? model.draft : ""),
  panes: (model) => model.desk.panes.map((pane) => pane.id),
  // A pane the learner opened, a new tab or a split, is a shell with no agent in it.
  screen: (model, paneId, cols) => (paneId === CONDUCTOR ? conductorScreen(model, cols) : paneId === WORKER ? workerScreen(model) : paneId === CONFIG_WORKER ? configScreen(model) : shellScreen(PLACE)),
  // The mission's parts stand for pull requests, which could pass for real ones; the mark sits on the title, clear of the card's marks.
  examples: ["[data-mission-node] > span:first-child > span[title]"],
};
