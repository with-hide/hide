// Chapter 0's world: one job carried from the learner's request to the report,
// as a mission (`hide-mission`), at the pace an agent works. The job is moving
// sign-in to email links: a survey, a token table, the API on it, the sign-in
// screen and the mail in parallel, checked together at the end, two workers at
// a time. Whatever words the learner sends play the next part of the script,
// since a practice understands no words, and the Conductor's own turns end by
// themselves (`later`): the first words are the mission, which the Conductor
// plans for a few seconds before the plan stands on the map and it asks
// whether to start; the next words approve it, and the Conductor starts each
// part's worker in a tab of its own as the part's turn and the cap allow. A
// while in, it asks one decision (`hide mission decide`) with its choices,
// whether to keep password sign-in, which reaches the sidebar's 내 차례; a
// choice pressed on the map's banner, an answer written there, or the next
// words said to the Conductor answer it, and the answer changes the plan:
// keeping it gives the screen both ways, dropping it adds a part that takes
// password sign-in out. The rest runs, the check passes, the Conductor
// reports, the Project's Memory keeps what was decided, and the Explorer
// marks the files the parts changed, any of which opens beside it. Every word the
// world shows is in the learner's language (`practice.mission.world.*`), since
// this is the chapter a newcomer meets first.

import type { ProjectTasksScope } from "../../agentScope";
import type { MessageKey } from "../../i18n/catalogs";
import { clientI18n } from "../../i18n/translator";
import { nextSidebarLens, type AgentChip, type AgentRow, type AgentState, type ChangesSnapshot, type DetailItem, type EditorDocumentSnapshot, type EditorTabSnapshot, type MemorySection, type Mission, type MissionNode, type PaneFocusRequest, type PaneHeader, type SidebarLens, type ViewDisplaySnapshot } from "../../snapshot";
import type { DirectoryList } from "../../store";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, counted, deskSnapshot, focusedPane, hideAiOn, keyText, shellScreen, typed, withLineage, withTasks, type DeskModel, type PracticePlace } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };
/** The checkout's folder, as the kit draws it. */
const ROOT = `/work/${PLACE.project}`;
/** The sign-in screen the screen worker made, the file chapter 0 opens from the Explorer. */
export const SIGN_IN_FILE = `${ROOT}/web/src/pages/SignIn.tsx`;

/** The Conductor's pane. */
export const CONDUCTOR = "cd";

const MISSION_ID = "practice-mission";

/**
 * Where the mission stands: 0 before it, 1 the Conductor planning, 2 the plan
 * up and waiting for the learner's go, 3 the survey, 4 the token table and the
 * screen, 5 the API and the screen with a decision waiting, 6 answered, 7 the
 * mail (and, when dropped, taking password sign-in out), 8 the check, 9 done
 * and reported.
 */
export type MissionStage = 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9;

/** The stage the decision waits at, and the last. */
const ASKING: MissionStage = 5;
const DONE: MissionStage = 9;

/** How long each of the Conductor's own turns takes before the next stage, by the stage it ends. */
export const BEATS: Partial<Record<MissionStage, number>> = { 1: 3500, 3: 3000, 4: 4000, 6: 3000, 7: 3500, 8: 3000 };
/** Between two lines the Conductor writes. */
const LINE_MS = 450;
/** The event a Conductor's turn ending plays: the stage it moves to. */
const BEAT = "practice_beat";

type Part = "survey" | "db" | "screen" | "api" | "mail" | "drop" | "verify";

/** One part of the plan: its kind, who does it in which pane, when it starts and ends, its PR, what it waits on, and the file it touches. */
type PartSpec = {
  key: Part; kind: "change" | "research" | "verify"; pane: string; agent: "claude" | "codex"; model: string; effort: "low" | "medium" | "high";
  start: MissionStage; end: MissionStage; pr: number | null; deps: Part[]; file: string;
};

const PARTS: PartSpec[] = [
  { key: "survey", kind: "research", pane: "sv", agent: "claude", model: "sonnet", effort: "medium", start: 3, end: 4, pr: null, deps: [], file: "src/auth/session.ts" },
  { key: "db", kind: "change", pane: "db", agent: "codex", model: "gpt-5.5", effort: "medium", start: 4, end: 5, pr: 31, deps: ["survey"], file: "db/migrations/0042_login_tokens.sql" },
  { key: "screen", kind: "change", pane: "ui", agent: "claude", model: "sonnet", effort: "medium", start: 4, end: 7, pr: 33, deps: ["survey"], file: "web/src/pages/SignIn.tsx" },
  { key: "api", kind: "change", pane: "ap", agent: "codex", model: "gpt-5.5", effort: "high", start: 5, end: 7, pr: 32, deps: ["db"], file: "src/auth/magicLink.ts" },
  { key: "mail", kind: "change", pane: "ml", agent: "codex", model: "gpt-5.5", effort: "low", start: 7, end: 8, pr: 34, deps: ["survey"], file: "mail/templates/sign-in.html" },
  { key: "drop", kind: "change", pane: "pw", agent: "codex", model: "gpt-5.5", effort: "medium", start: 7, end: 8, pr: 35, deps: ["api"], file: "src/auth/password.ts" },
  { key: "verify", kind: "verify", pane: "vf", agent: "claude", model: "sonnet", effort: "medium", start: 8, end: 9, pr: null, deps: ["api", "screen", "mail"], file: "e2e/sign-in.spec.ts" },
];

/** One of the world's words in the learner's language. */
const say = (key: string, values?: Record<string, string | number>) => clientI18n.t(`practice.mission.world.${key}` as MessageKey, values as never) as string;
const title = () => say("title");
const decision = () => say("decision");
/** The decision's choices, as `hide mission decide --option` gives them; the first, keeping password sign-in, is the one recommended. */
const options = () => [say("option1"), say("option2")];

export type MissionModel = {
  desk: DeskModel;
  stage: MissionStage;
  /** When each stage began, which paces the Conductor's lines and its next beat. */
  times: Partial<Record<MissionStage, number>>;
  /** What the learner has typed into the Conductor's prompt and not sent yet. */
  draft: string;
  /** The mission, the go and the answer, in the learner's words. */
  said: string[];
  /** Anything else said to the Conductor, which it takes on as it goes. */
  noted: string[];
  /** Whether the answer drops password sign-in, which adds the part that takes it out. */
  drop: boolean;
  /** The 보기 slot: what it opened, oldest first, and whether the lane draws it. */
  detail: DetailItem[];
  detailShown: boolean;
  /** The core's count of lane calls; an open in the 보기 slot is one, which brings an unpinned lane up. */
  laneCalls: number;
  /** Which list the sidebar shows (`sidebar_lens_set`). */
  lens: SidebarLens;
  /** The lane's tool beside the 보기 slot: the Project's Memory or the Explorer. */
  tool: "memory" | "explorer" | null;
  /** The Explorer's open folders. */
  expanded: string[];
  /** The file opened from the Explorer, in the lane's View. */
  opened: string | null;
  /** Whether the learner opened the Conductor while it asked, from 내 차례 or anywhere else. */
  looked: boolean;
  /** The core's receipt for the last focus asked by relationship. */
  focus: PaneFocusRequest | null;
};

/** Whether the lane shows the Memory tool. */
export const showsMemory = (model: MissionModel) => model.tool === "memory";
/** Whether the lane shows the Explorer. */
export const showsExplorer = (model: MissionModel) => model.tool === "explorer";
/** The file open in the lane's View. */
export const openedFile = (model: MissionModel) => model.opened;

/** The pane holding the keyboard. */
export const focusedOn = (model: MissionModel) => focusedPane(model.desk);

/** Whether the 보기 slot shows this chapter's mission map in front. */
export function missionInSlot(model: MissionModel): boolean {
  const top = model.detail.at(-1);
  return model.detailShown && top?.kind === "mission" && top.mission === MISSION_ID;
}

const since = (model: MissionModel, stage: MissionStage) => model.times[stage] ?? Date.now();

/** The parts in the plan: dropping password sign-in adds its part, and the check then waits on it too. */
function parts(model: MissionModel): PartSpec[] {
  return PARTS.filter((part) => part.key !== "drop" || model.drop).map((part) => (part.key === "verify" && model.drop ? { ...part, deps: [...part.deps, "drop" as const] } : part));
}

/** Where a part stands once the mission runs: waiting its turn, at work, or done. */
function phase(part: PartSpec, stage: MissionStage): "waiting" | "working" | "done" {
  return stage < part.start ? "waiting" : stage < part.end ? "working" : "done";
}

/** The parts whose workers have started, in the order they started. */
const begun = (model: MissionModel) => (model.stage >= 3 ? parts(model).filter((part) => model.stage >= part.start) : []);

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
function asking(row: AgentRow, what: string, at: number): AgentRow {
  const state: AgentState = {
    ...row.state,
    attention: true, needs_you: true, title_emphasized: true, asking: true, working: false,
    chip_tone: tone("warning", false), mark_tone: tone("warning", false), bucket: "turn", attention_rank: 1, link_rank: 0, search_tone: "attention", subtree: "waiting", link: "question",
    verb: "answer", tree_rank: 0, request_since: at,
    session: { group: "needs_you", line: null, unfinished: false },
    ask: { verb: "answer", what, more: 0 },
    line: { text: what, mode: "request", tone: tone("warning", false) },
  };
  return {
    ...row, state, group: "needs_you", mark: "turn", status_code: "question", demand: "question", activity: "idle", emphasized: true, unread: true,
    user_turn: { kind: "question", content: null }, changed_at_unix_ms: at,
  };
}

const COUNTS = (working: number) => ({ error: 0, approval: 0, question: 0, working, done: 0 });

/** Whether the Conductor waits on the learner: its go, then its decision. */
const asks = (stage: MissionStage) => stage === 2 || stage === ASKING;
/** Whether the Conductor is at work: planning, or watching its workers. */
const conductorWorks = (stage: MissionStage) => stage !== 0 && stage !== DONE && !asks(stage);

/** The workers at work. */
const busy = (model: MissionModel) => begun(model).filter((part) => phase(part, model.stage) === "working").length;

/** The Conductor as the stage has it. */
function conductorRow(model: MissionModel): AgentRow {
  const working = conductorWorks(model.stage);
  const base = agent({
    pane_id: CONDUCTOR, identity_label: title(), agent_kind: "claude", session_id: "practice-conductor",
    group: working ? "working" : "seen", mark: working ? "going" : "quiet", status_code: working ? "working" : "idle", activity: working ? "working" : "idle", elapsed: working ? "1s" : "8m",
  });
  const row = model.stage === 2 ? asking(base, say("approveQuestion"), since(model, 2)) : model.stage === ASKING ? asking(base, decision(), since(model, ASKING)) : working ? base : resting(base);
  const children = begun(model).map((part) => part.pane);
  if (!children.length) return row;
  return { ...row, lineage_child_pane_ids: children, lineage_collapsed: false, direct_child_counts: COUNTS(busy(model)), descendant_counts: COUNTS(busy(model)) };
}

/** A part's worker, delegated by the Conductor, at work until its part is done. */
function workerRow(part: PartSpec, stage: MissionStage): AgentRow {
  const base: AgentRow = {
    ...agent({ pane_id: part.pane, identity_label: say(`node.${part.key}`), agent_kind: part.agent, session_id: `practice-worker-${part.key}`, group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "2m" }),
    delegated: true, lineage_parent_pane_id: CONDUCTOR, lineage_path_pane_ids: [CONDUCTOR], lineage_depth: 1,
  };
  const row = phase(part, stage) === "done" ? resting(base) : base;
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
    // Past every worker's tab, so a tab or split the learner opens takes a new number.
    nextId: PARTS.length + 2,
    operations: [],
  };
  // The sidebar starts on its 태스크 list, where a question lands in 내 차례.
  return { desk, stage: 0, times: { 0: Date.now() }, draft: "", said: [], noted: [], drop: false, detail: [], detailShown: false, laneCalls: 0, lens: "tasks", tool: null, expanded: [`${ROOT}/web`, `${ROOT}/web/src`, `${ROOT}/web/src/pages`], opened: null, looked: false, focus: null };
}

/** The desk with a tab for each worker that has started and has none yet. */
function withWorkerTabs(model: MissionModel): DeskModel {
  const desk = model.desk;
  const fresh = begun(model).filter((part) => !desk.panes.some((pane) => pane.id === part.pane));
  if (!fresh.length) return desk;
  const tabId = (part: PartSpec) => `t${PARTS.findIndex((one) => one.key === part.key) + 2}`;
  return {
    ...desk,
    panes: [...desk.panes, ...fresh.map((part) => ({ id: part.pane, agent: null, title: say(`node.${part.key}`) }))],
    tabs: [...desk.tabs, ...fresh.map((part) => ({ id: tabId(part), name: null, root: { type: "pane" as const, pane_id: part.pane }, focused: part.pane, zoomed: false }))],
    groups: desk.groups.map((group) => (group.id === null ? { ...group, tabs: [...group.tabs, ...fresh.map(tabId)] } : group)),
  };
}

/** The desk with each pane's agent as the stage has it. */
function withAgents(model: MissionModel): DeskModel {
  const rows: Record<string, AgentRow> = { [CONDUCTOR]: conductorRow(model) };
  for (const part of begun(model)) rows[part.pane] = workerRow(part, model.stage);
  return { ...model.desk, panes: model.desk.panes.map((pane) => ({ ...pane, agent: rows[pane.id] ?? pane.agent })) };
}

// --- The mission ---------------------------------------------------------------------

const PR_URL = (number: number) => `https://example.com/${PLACE.project}/pull/${number}`;
const MINUTE = 60_000;

const NODE: Omit<MissionNode, "key" | "title" | "status" | "worker" | "kind" | "depends_on" | "delivery"> = {
  blocked: null, agent_id: null, pane_id: null, machine: null,
  pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 1,
  line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, steps: [], brief_bytes: null, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, landing: null, verdict: null, checks: null, plan: null, prs: [], attention: null, hide: null,
};

/** A part as the record holds it at the model's stage. */
function node(model: MissionModel, part: PartSpec): MissionNode {
  const at = model.stage === 2 ? "waiting" : phase(part, model.stage);
  const base: MissionNode = {
    ...NODE, key: part.key, title: say(`node.${part.key}`), kind: part.kind, depends_on: part.deps, delivery: part.kind === "change" ? "pr" : null,
    status: at === "waiting" ? "planned" : at === "working" ? "working" : "done",
    worker: { agent: part.agent, model: part.model, effort: part.effort },
  };
  // The screen's part changes with the answer: both ways kept, or the password field gone.
  const change = part.key === "screen" && model.stage > ASKING ? say(model.drop ? "screenChangeDrop" : "screenChangeKeep") : part.key === "drop" ? say("dropChange") : null;
  if (at === "waiting") return { ...base, change };
  const branch = part.kind === "change" ? `practice/sign-in-${part.key}` : null;
  const started = { agent_id: `agent-${part.pane}`, pane_id: part.pane, branch, started_at_unix_ms: since(model, part.start), change };
  if (at === "working") return { ...base, ...started, stage: part.kind === "change" ? 1 : null, line: say(`line.${part.key}`) };
  const finished = { ...started, finished_at_unix_ms: since(model, part.end), line: say(`doneLine.${part.key}`) };
  if (part.kind === "verify") return { ...base, ...finished, verdict: "pass", checks: { passed: 12, total: 12 } };
  if (part.pr === null) return { ...base, ...finished };
  return { ...base, ...finished, pr: part.pr, prs: [{ number: part.pr, state: "mergeable", url: PR_URL(part.pr) }], pr_state: "mergeable", pr_url: PR_URL(part.pr), stage: 3, landing: { state: "left", action: "merge" } };
}

/** The mission once its plan stands; while the Conductor still plans there is none. */
function mission(model: MissionModel): Mission | null {
  if (model.stage < 2) return null;
  const now = Date.now();
  const status = model.stage === 2 ? "planning" : model.stage === DONE ? "merge_wait" : "running";
  const plan = parts(model);
  const landed = plan.filter((part) => part.pr !== null && model.stage >= part.end).map((part) => part.key);
  return {
    id: MISSION_ID, title: title(), workspace_id: PLACE.project, project: PLACE.label, checkout_path: `/work/${PLACE.project}`, device_id: "local",
    status, scope: "pr", cap: 2, exempt: false, strength: "normal",
    // No session: this practice keeps no record to open.
    conductor: { pane_id: CONDUCTOR, name: title(), agent_kind: "claude", session: null, present: true, takes_answers: true },
    nodes: plan.map((part) => node(model, part)), merge_order: plan.filter((part) => part.pr !== null).sort((a, b) => a.pr! - b.pr!).map((part) => part.key), landing_order: landed, stack_waiting: 0, check: "pnpm test:e2e", github: "practice-bot",
    decision: model.stage === ASKING ? decision() : null,
    question: model.stage === ASKING ? { key: 1, why: say("why"), options: options(), recommend: 1, node: "screen", asked_at_unix_ms: since(model, ASKING) } : null,
    answer: null, answer_failed: null, delay: null, report: model.stage === DONE ? say("report", { count: landed.length }) : null,
    created_at_unix_ms: since(model, 1), started_at_unix_ms: model.stage >= 3 ? since(model, 3) : null, closed_at_unix_ms: null,
    running: busy(model), stale: false, github_stale: false, github_read_at_unix_ms: now,
  };
}

// --- The Conductor's prompt and its turns ---------------------------------------------------

/** The model moved to `stage`, which begins now, with a tab for each worker it starts. */
function to(model: MissionModel, stage: MissionStage): MissionModel {
  const moved = { ...model, stage, times: { ...model.times, [stage]: Date.now() } };
  return { ...moved, desk: withWorkerTabs(moved) };
}

/** The decision answered with `words`, dropping password sign-in or keeping it: the Conductor changes the plan and goes back to work. */
function answered(model: MissionModel, words: string, drop: boolean): MissionModel {
  return to({ ...model, said: [...model.said, words], drop }, 6);
}

/**
 * The prompt, sent: the first words are the mission, the next the go once the
 * plan is up, the next the decision's answer (which keeps password sign-in
 * unless it is the choice that drops it); words said while the Conductor is at
 * work, or after, it takes on as it goes. An empty prompt sends nothing.
 */
function submit(model: MissionModel): MissionModel {
  const words = model.draft.trim();
  if (!words) return { ...model, draft: "" };
  const sent = { ...model, draft: "", said: [...model.said, words] };
  switch (model.stage) {
    case 0:
      return to(sent, 1);
    case 2:
      return to(sent, 3);
    case ASKING:
      return answered({ ...model, draft: "" }, words, words === options()[1]);
    default:
      return { ...model, draft: "", noted: [...model.noted, words] };
  }
}

/**
 * The banner's answer, as the core types it into the Conductor's conversation:
 * the question, then the choice pressed or the words written. An answer to
 * another mission or question, or once nothing is asked, is refused.
 */
function answerOf(model: MissionModel, payload: Record<string, unknown>): { line: string; drop: boolean } | null {
  if (model.stage !== ASKING || payload.mission !== MISSION_ID || payload.key !== 1) return null;
  const answer = typeof payload.option === "number" ? options()[payload.option - 1] : typeof payload.text === "string" ? payload.text.trim() : undefined;
  return answer ? { line: say("answerLine", { question: decision(), answer }), drop: payload.option === 2 } : null;
}

/** The Conductor's turn that ends on its own: the plan stands, a part finishes and the next starts, the question comes, the check passes. */
function later(model: MissionModel): { at: number; event: CoreEvent } | null {
  const wait = BEATS[model.stage];
  return wait === undefined ? null : { at: since(model, model.stage) + wait, event: { kind: BEAT, payload: { stage: model.stage + 1 } } };
}

// --- The events --------------------------------------------------------------------------

/** A focus moved, by a click, a key, a 내 차례 row or a relationship; opening the Conductor while it asks is looking at the question, and coming back from a worker by its head is not. */
function focused(model: MissionModel, event: CoreEvent): MissionModel | null {
  const desk = applyDesk(model.desk, event);
  const paneId = String(event.payload.pane_id);
  const requestId = event.payload.request_id;
  const focus: PaneFocusRequest | null = typeof requestId === "string" ? { request_id: requestId, target_pane_id: paneId, phase: "succeeded", message: null, retryable: false } : model.focus;
  const looked = model.looked || (model.stage === ASKING && paneId === CONDUCTOR && typeof requestId !== "string");
  if (!desk && looked === model.looked && focus === model.focus) return null;
  return { ...model, desk: desk ?? model.desk, focus, looked };
}

function apply(model: MissionModel, event: CoreEvent): MissionModel | null {
  const p = event.payload;
  switch (event.kind) {
    case BEAT:
      return p.stage === model.stage + 1 && BEATS[model.stage] !== undefined ? to(model, p.stage as MissionStage) : null;
    // Only the Conductor is talked to here; keys typed into another pane reach no one.
    case "key":
      return p.pane_id === CONDUCTOR && typeof p.bytes_base64 === "string" ? typed(model, keyText(p.bytes_base64), submit) : null;
    case "mission_answer": {
      const answer = answerOf(model, p);
      return answer ? answered(model, answer.line, answer.drop) : null;
    }
    case "focus_pane":
      return focused(model, event);
    // The map opens in the 보기 slot as a lane call, the way the core numbers one; a record or a card this world keeps none of.
    case "detail_open": {
      const item = p.item as DetailItem;
      if (item.kind !== "mission" || item.mission !== MISSION_ID || model.stage < 2) return null;
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
    // The Memory tool shown, by ⇧⌘L, its rail icon or from elsewhere.
    case "project_memory_navigate":
      return p.workspace_id === PLACE.project && model.tool !== "memory" ? { ...model, tool: "memory" } : null;
    // A tool shown or taken down, a path revealed in the Explorer, or the lane put away with what it showed.
    case "workspace_view": {
      let tool = model.tool;
      if ("tool" in p) tool = p.tool === "memory" || p.tool === "explorer" ? p.tool : null;
      if (typeof p.reveal === "string") tool = "explorer";
      const opened = p.open === false ? null : model.opened;
      if (p.open === false) tool = null;
      return tool === model.tool && opened === model.opened ? null : { ...model, tool, opened };
    }
    // The Explorer folds a folder through the core's ui state.
    case "ui_state_update": {
      if (!Array.isArray(p.expanded_paths)) return null;
      const expanded = p.expanded_paths.filter((path): path is string => typeof path === "string");
      return expanded.join("\n") === model.expanded.join("\n") ? null : { ...model, expanded };
    }
    // A file of the checkout opened in the lane's View, as a lane call; the map steps aside for it.
    case "file_open": {
      const path = String(p.path);
      if (p.workspace_id !== PLACE.project || !FILES.includes(path.slice(ROOT.length + 1))) return null;
      return path === model.opened ? null : { ...model, opened: path, detailShown: false, laneCalls: model.laneCalls + 1 };
    }
    default: {
      const desk = applyDesk(model.desk, event);
      return desk ? { ...model, desk } : null;
    }
  }
}

// --- The snapshot ------------------------------------------------------------------------

/** The Conductor pane's head as the core sends it: asking for its go or its decision, at work, or at rest. */
function conductorHeader(model: MissionModel): PaneHeader {
  const band = (reason: string, at: number) => ({ kind: "answer" as const, reason, since_unix_ms: at, read: false, action: null, more: 0 });
  if (model.stage === 2) return { working: false, notice: null, band: band(say("approveQuestion"), since(model, 2)) };
  if (model.stage === ASKING) return { working: false, notice: null, band: band(decision(), since(model, ASKING)) };
  if (conductorWorks(model.stage)) return { working: true, notice: null, band: null, request: { text: model.said.at(-1)!, images: 0 } };
  return { working: false, notice: null, band: null };
}

/** The 태스크 list's one task: the Conductor with its workers, in 내 차례 while it asks. */
function tasks(model: MissionModel, nowMs: number): ProjectTasksScope {
  const state = asks(model.stage) ? "needs_you" as const : conductorWorks(model.stage) ? "working" as const : "resting" as const;
  const members = [CONDUCTOR, ...begun(model).map((part) => part.pane)];
  const recency = new Date(nowMs).toISOString();
  return { cards: [{ root: CONDUCTOR, state, members, checkouts: [`${PLACE.project}:${PLACE.branch}`], recency, stage: 1 }], needs_you: state === "needs_you" ? 1 : 0, recency };
}

function snapshot(model: MissionModel, nowMs: number): WorldSnapshot {
  const desk = withAgents(model);
  const drawn = deskSnapshot([{ place: PLACE, model: desk }], PLACE.project, nowMs);
  const children = begun(model).map((part) => part.pane);
  // Once the mission runs, its workers are drawn under the Conductor, and every agent not at work is counted where it stands.
  const lineage = children.length ? withLineage(drawn.rest, CONDUCTOR, children) : drawn.rest;
  const doneWorkers = begun(model).filter((part) => phase(part, model.stage) === "done").length;
  const restingCount = doneWorkers + (model.stage === 0 || model.stage === DONE ? 1 : 0);
  const asked = asks(model.stage) ? counted(lineage, "needs_you") : lineage;
  const counts = restingCount ? counted(asked, "seen", restingCount) : asked;
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
      navigator: { ...navigator, workspaces, changes_root_path: ROOT },
      missions: { items: current ? [current] : [] },
      // Opening the 보기 slot opens the lane, as the core does on `detail_open`; the tool stands beside it, and an opened file fills the View.
      workspace_view: viewOf(model, base.workspace_view!),
      ui_state: { ...base.ui_state!, sidebar_lens: model.lens, detail: { items: model.detail, shown }, expanded_paths: model.expanded },
      terminal: { headers },
      // Hide AI on, so the Memory tool reads as it does for an operator who keeps one; the relationship focus answered.
      status: { ...base.status, pane_focus_request: model.focus, background_ai: { enabled: true, provider: "claude", chosen: true, agent_summary: true, providers: [], unavailable_reason: null } },
      // The core's proof of each agent's session, which draws each one on the Overview's map.
      overview_graph: {
        available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 30 * 24 * 60 * MINUTE, letters: [],
        sessions: agents.flatMap((row) => (row.session_id ? [{ device_id: "local", pane_id: row.pane_id, session_digest: row.session_id, launch_model: null, launch_effort: null }] : [])),
      },
    },
    frames: { memory: memory(model, nowMs), suggestions: hideAiOn(PLACE.project), listings: listings(), changes: changes(model), ...opened(model) },
  };
}

// --- The checkout's files ------------------------------------------------------------------

/** Every file of the checkout, relative to it: the code each part reads or changes, and a few more. */
const FILES = ["README.md", "package.json", "db/migrations/0041_sessions.sql", "e2e/sign-in.spec.ts", "mail/templates/sign-in.html", "src/auth/magicLink.ts", "src/auth/password.ts", "src/auth/session.ts", "web/src/pages/Home.tsx", "web/src/pages/SignIn.tsx"].concat(PARTS.map((part) => part.file)).filter((file, at, all) => all.indexOf(file) === at);

/** Each folder's listing as the core answers it, folders first. */
function listings(): Record<string, DirectoryList> {
  const present = FILES;
  const folders = new Set([""]);
  for (const file of present) file.split("/").slice(0, -1).forEach((_, at, parts) => folders.add(parts.slice(0, at + 1).join("/")));
  return Object.fromEntries([...folders].map((folder) => {
    const path = folder ? `${ROOT}/${folder}` : ROOT;
    const inside = (name: string) => (folder ? name.startsWith(`${folder}/`) : true);
    const subfolders = [...folders].filter((one) => one && inside(one) && one.split("/").length === (folder ? folder.split("/").length + 1 : 1));
    const files = present.filter((file) => inside(file) && file.split("/").length === (folder ? folder.split("/").length + 1 : 1));
    const entries = [...subfolders.sort().map((one) => ({ name: one.split("/").at(-1)!, path: `${ROOT}/${one}`, is_directory: true })), ...files.sort().map((one) => ({ name: one.split("/").at(-1)!, path: `${ROOT}/${one}`, is_directory: false }))];
    return [path, { kind: "file_list" as const, root_path: path, entries, truncated: false }];
  }));
}

/** The lines each part adds and removes, and whether its file is new. */
const CHANGED: Partial<Record<Part, { added: number; removed: number; fresh: boolean }>> = {
  db: { added: 24, removed: 0, fresh: true }, screen: { added: 38, removed: 21, fresh: false }, api: { added: 57, removed: 0, fresh: true },
  mail: { added: 19, removed: 0, fresh: false }, drop: { added: 3, removed: 64, fresh: false },
};

/** What the finished parts changed, uncommitted here, since the practice keeps every worker in this one checkout. */
function changes(model: MissionModel): ChangesSnapshot {
  const done = parts(model).filter((part) => part.kind === "change" && phase(part, model.stage) === "done");
  return {
    root_path: ROOT,
    entries: done.map((part) => ({ path: `${ROOT}/${part.file}`, relative_path: part.file, previous_relative_path: null, status: CHANGED[part.key]!.fresh ? "untracked" as const : "modified" as const, added_lines: CHANGED[part.key]!.added, removed_lines: CHANGED[part.key]!.removed })),
    committed: [], base_branch: "main", selected_path: null, selected_committed: false, diff: null, unavailable_reason: null,
  };
}

const EDITOR_TAB = "file1";

/** The lane as the core draws it: the 보기 slot, the tool beside it, and the opened file as the View's one display. */
function viewOf(model: MissionModel, view: NonNullable<WorldSnapshot["rest"]["workspace_view"]>): NonNullable<WorldSnapshot["rest"]["workspace_view"]> {
  const shown = model.detailShown && model.detail.length > 0;
  const lane = { ...view, open: shown || model.tool !== null || model.opened !== null, tool: model.tool, lane_called: model.laneCalls, lane_calls: model.laneCalls, lane_called_side: "view" as const };
  if (!model.opened) return lane;
  const display: ViewDisplaySnapshot = { id: "view1", tab_id: EDITOR_TAB, path: model.opened, label: model.opened.split("/").at(-1)!, kind: "file", committed: null, preview: true, state: "open", reason: null };
  return { ...lane, documents: true, layout: { root: { area: { id: "v1", active: display.id, displays: [display] } }, active_area: "v1", display_count: 1, limits: { areas: 4, depth: 3, displays: 32 } } };
}

/** The opened file's editor tab and its contents. */
function opened(model: MissionModel): { editor?: { tabs: EditorTabSnapshot[]; active_tab_id: string }; documents?: Record<string, EditorDocumentSnapshot> } {
  const path = model.opened;
  if (!path) return {};
  const relative = path.slice(ROOT.length + 1);
  const label = relative.split("/").at(-1)!;
  const tab: EditorTabSnapshot = { id: EDITOR_TAB, device_id: "local", workspace_id: PLACE.project, checkout_id: `${PLACE.project}:${PLACE.branch}`, path, label, kind: "file", diff_committed: null, markdown_live: true, wrap: false, dirty: false, preview: true };
  const language = relative.endsWith(".md") ? "markdown" : relative.endsWith(".sql") ? "sql" : relative.endsWith(".html") ? "html" : "typescript";
  const document: EditorDocumentSnapshot = { path, language, document_kind: relative.endsWith(".md") ? "markdown" : "text", contents_utf8: contents(model, relative), revision: "sha256:0", dirty: false, readonly_reason: null, conflict: null, save: null };
  return { editor: { tabs: [tab], active_tab_id: EDITOR_TAB }, documents: { [EDITOR_TAB]: document } };
}

/** What a file holds: the sign-in screen as the screen worker left it, both ways or the link alone; any other file a line saying what it is. */
function contents(model: MissionModel, relative: string): string {
  if (relative !== "web/src/pages/SignIn.tsx") return `// ${relative}\n`;
  const finished = model.stage >= 7;
  return [
    'import { useState } from "react";',
    'import { requestLink } from "../api/magicLink";',
    "",
    "export function SignIn() {",
    '  const [email, setEmail] = useState("");',
    "  const [sent, setSent] = useState(false);",
    '  if (sent) return <p>Check your inbox for a sign-in link.</p>;',
    "  return (",
    "    <form onSubmit={async (event) => { event.preventDefault(); await requestLink(email); setSent(true); }}>",
    '      <input type="email" value={email} onChange={(event) => setEmail(event.target.value)} />',
    '      <button type="submit">Email me a link</button>',
    ...(finished && !model.drop ? ['      <a href="/sign-in/password">Use a password instead</a>'] : []),
    "    </form>",
    "  );",
    "}",
    "",
  ].join("\n");
}

// --- The Project's Memory ------------------------------------------------------------------

const DAY_MS = 24 * 60 * MINUTE;

/** The card the learner's answer becomes once the work is done, the newest in the Project's Memory. */
export const DECISION_CARD = 3;

/**
 * What the Project's Memory keeps: two older memories, and once the mission is
 * done the one Hide AI made of the decision, newest first, from the learner's own words.
 */
function memory(model: MissionModel, nowMs: number): MemorySection {
  type Card = [number: number, kind: "decision" | "rule" | "lesson", key: string, ageMs: number, mine: number];
  const decided: Card[] = model.stage === DONE ? [[DECISION_CARD, "decision", model.drop ? "memoryDrop" : "memoryKeep", MINUTE, 1]] : [];
  const cards: Card[] = [...decided, [2, "lesson", "memoryLesson", 3 * DAY_MS, 0], [1, "rule", "memoryRule", 6 * DAY_MS, 1]];
  return {
    project: {
      workspace_id: PLACE.project, unavailable: null, loading: false, enabled: true, capacity_reached: false, evicted_cards: 0, evicted_through_unix_ms: null,
      rows: cards.map(([number, kind, key, ageMs, mine], index) => ({
        number, title: say(key), kind, human_source_count: mine, provided_session_count: index, pull_count: index, updated_at_unix_ms: nowMs - ageMs, verifying: false, promotion: null,
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

/** The Conductor's lines for a stage, written one by one from when the stage began, as an agent writes; what the learner said shows at once. */
function written(model: MissionModel, stage: MissionStage, lines: string[]): string[] {
  const shown = Math.floor((Date.now() - since(model, stage)) / LINE_MS) + 1;
  return lines.slice(0, Math.max(0, shown));
}

const ran = (command: string, result: string) => [`● ${BOLD}Bash${RESET}(${command})`, `  ${DIM}⎿${RESET}  ${result}`];

/** What the Conductor writes when a stage begins: parts done, workers started, the decision, the changed plan, the check, the report. */
function stageLines(model: MissionModel, stage: MissionStage): string[] {
  const plan = parts(model);
  const finished = plan.filter((part) => part.end === stage);
  const starting = plan.filter((part) => part.start === stage);
  const done = finished.map((part) => (part.pr === null
    ? `● ${GREEN}✓${RESET} ${say(`node.${part.key}`)} · ${say(`doneLine.${part.key}`)}`
    : `● ${GREEN}#${part.pr}${RESET} ${say(`node.${part.key}`)} · ${say("ciPassed")}`));
  const spawn = starting.length ? ran(`hide agent spawn --parent here --plan-node ${starting.map((part) => part.key).join(",")} …`, say("spawned", { parts: starting.map((part) => say(`node.${part.key}`)).join(", ") })) : [];
  switch (stage) {
    case 1:
      return [`● ${say("reading")}`, ...ran(`hide mission create --title "${title()}"`, say("made"))];
    case 2:
      return [...ran("hide mission plan --file plan.json", say("planned", { count: PARTS.length - 1 })), `  ${DIM}${say("planShape")}${RESET}`, "", `● ${YELLOW}${say("planReady")}${RESET}`];
    case 3:
      return [`● ${say("starting")}`, ...spawn];
    case ASKING:
      return [...done, ...spawn, "", `● ${YELLOW}${say("decideHead")}${RESET} ${say("decideWhy")}`, `  ${decision()}`, ...ran("hide mission decide --option … --recommend 1 --node screen", say("asked"))];
    case 6:
      return [...ran("hide mission plan --file plan.json", say(model.drop ? "foldedDrop" : "foldedKeep")), ...ran("hide request send ui …", say("toldScreen"))];
    case DONE:
      return [...done, "", ...ran("hide mission report …", say("reported")), `● ${GREEN}${say("report", { count: plan.filter((part) => part.pr !== null).length })}${RESET}`];
    default:
      return [...done, ...spawn];
  }
}

/** The Conductor's conversation, stage by stage. */
function conductorScreen(model: MissionModel, cols: number): string {
  const said = (words: string) => ["", `${DIM}>${RESET} ${words}`, ""];
  const spinner = (words: string) => ["", `${CLAUDE}✻ ${words} ${DIM}(esc to interrupt)${RESET}`];
  const rows = ["", "", `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`];
  const heard: Partial<Record<MissionStage, string>> = { 1: model.said[0], 3: model.said[1], 6: model.said[2] };
  for (let stage = 1 as MissionStage; stage <= model.stage; stage = (stage + 1) as MissionStage) {
    const words = heard[stage];
    if (words) rows.push(...said(words));
    rows.push(...written(model, stage, stageLines(model, stage)));
  }
  for (const words of model.noted) rows.push(...said(words), `● ${say("noted")}`);
  if (model.stage === 1) rows.push(...spinner(say("thinking")));
  else if (conductorWorks(model.stage)) rows.push(...spinner(say("waiting", { count: busy(model) })));
  rows.push("", rule(cols), `${DIM}>${RESET} ${model.draft}`, rule(cols));
  return rows.join("\r\n");
}

/** A worker's terminal: its brief, the file it changed, the Conductor's letter for the screen's worker, and where it stands. */
function workerScreen(model: MissionModel, part: PartSpec): string {
  const color = part.agent === "codex" ? CODEX : CLAUDE;
  const head = part.agent === "codex" ? `${BOLD}>_ Codex${RESET}` : `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`;
  const rows = [head, `  ${DIM}~/${PLACE.project}-${part.key}${RESET}`, "", `${color}›${RESET} ${say("brief", { title: title(), part: say(`node.${part.key}`) })}`, "", `${DIM}•${RESET} ${BOLD}${part.kind === "research" ? "Read" : "Edited"}${RESET} ${part.file}`];
  if (part.key === "screen" && model.stage > ASKING) rows.push("", `${color}›${RESET} ${YELLOW}🦉  ${say("letterFrom", { title: title() })}${RESET}`, `  ${say(model.drop ? "letterDrop" : "letterKeep")}`);
  const done = phase(part, model.stage) === "done";
  rows.push("", done ? `${GREEN}✓${RESET} ${part.pr === null ? say(`doneLine.${part.key}`) : `#${part.pr} · ${say("ciPassed")}`}` : `${DIM}•${RESET} ${say(`line.${part.key}`)}`);
  return rows.join("\r\n");
}

export const MISSION_WORLD: PracticeWorld<MissionModel> = {
  initial,
  apply,
  snapshot,
  later,
  prompt: (model, paneId) => (paneId === CONDUCTOR ? model.draft : ""),
  panes: (model) => model.desk.panes.map((pane) => pane.id),
  screen: (model, paneId, cols) => {
    if (paneId === CONDUCTOR) return conductorScreen(model, cols);
    const part = PARTS.find((one) => one.pane === paneId);
    // A pane the learner opened, a new tab or a split, is a shell with no agent in it.
    return part ? workerScreen(model, part) : shellScreen(PLACE);
  },
  // The mission's parts stand for pull requests, which could pass for real ones; the mark sits on the title, clear of the card's marks.
  examples: ["[data-mission-node] > span:first-child > span[title]"],
};
