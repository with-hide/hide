// The device Overview's invented world (PRD overview-device-tasks): three Git
// projects and a folder on This Mac, with tasks in every state the Tasks tab
// draws, a backlog of issues and memos, measured disks on a nearly full
// volume, and one session's record. Every value is made up; the core's
// projections are fixed here as data, never recomputed.
import type { AgentScope, TaskCardScope } from "../agentScope";
import type { AgentPullRequest, AgentRow, AgentState, BacklogItem, RowMark, Checkout, IssueWork, LinkSummaries, ProjectDisk, SessionRecord, SnapshotRest, Task, TaskSource, WorkSession, Workspace, WorktreeDisk } from "../snapshot";
import { EMPTY_GALLERY_SCOPE } from "./agentScopes";
import states from "./agentStates.json";
import { checkout, REFERENCE_FOLDS, workspace } from "./sceneData";

const MINUTE = 60_000;
/** The hide project's issue key and the way a request names it, built from the number. */
export const hideKey = (number: number) => `github:example/hide#${number}`;
const hideRef = (number: number) => `#${number}`;
const GB = 1024 ** 3;
const MB = 1024 ** 2;

type Look = "idle" | "working" | "asking" | "done";
const PRESET: Record<Look, number> = { idle: 0, working: 9, asking: 7, done: 8 };
const MARK: Record<Look, RowMark> = { idle: "quiet", working: "going", asking: "turn", done: "done" };
const GROUP: Record<Look, string> = { idle: "seen", working: "working", asking: "needs_you", done: "done" };

type PullState = "failed" | "pending" | "mergeable" | "merged";

function pull(repository: string, number: number, title: string, state: PullState): AgentPullRequest {
  return {
    number, title, url: `https://example.invalid/${repository}/pull/${number}`, badge: state === "merged" ? "merged" : "open",
    checks: state === "failed" ? "failed" : state === "pending" ? "pending" : "passing", review: state === "pending" ? "review_required" : "approved",
    head_branch: `agents/${number}`, closing_issues: [], live: true, duty: true, created: true, settled_at_unix_ms: state === "merged" ? 0 : null,
  };
}

type AgentSpec = {
  pane: string;
  title: string;
  look: Look;
  ago: number;
  kind?: string;
  parent?: string;
  children?: string[];
  ask?: AgentState["ask"];
  /** What the session is doing now, as its quiet sentence. */
  now?: string;
  pulls?: { pr: AgentPullRequest; state: PullState }[];
  approval?: boolean;
};

function agent(spec: AgentSpec, now: number): AgentRow {
  const preset = states.presets[PRESET[spec.look]] as AgentState;
  const pulls = spec.pulls ?? [];
  const state: AgentState = {
    ...preset,
    root: spec.parent === undefined,
    ask: spec.ask ?? null,
    line: spec.now ? { text: spec.now, mode: "quiet", tone: { kind: "subtle", read: false } } : preset.line,
    pr: pulls.length ? { count: pulls.length, worst: pulls[0]!.state, pulls: pulls.map((one, index) => ({ index, state: one.state })) } : null,
    request_since: now - spec.ago,
  };
  return {
    state,
    id: spec.pane,
    pane_id: spec.pane,
    identity_label: spec.title,
    agent_kind: spec.kind ?? "claude",
    session_id: `session-${spec.pane}`,
    mark: MARK[spec.look],
    group: GROUP[spec.look],
    status_code: spec.look === "asking" ? (spec.approval ? "approval" : "question") : spec.look === "working" ? "working" : spec.look === "done" ? "done" : "idle",
    demand: spec.look === "asking" ? (spec.approval ? "approval" : "question") : "none",
    activity: spec.look === "working" ? "working" : "idle",
    emphasized: spec.look === "asking" || spec.look === "done",
    unread: spec.look === "asking",
    changed_at_unix_ms: now - spec.ago,
    delegated: spec.parent !== undefined,
    lineage_parent_pane_id: spec.parent ?? null,
    lineage_depth: spec.parent ? 1 : 0,
    lineage_child_pane_ids: spec.children ?? [],
    request: { verb: spec.look === "working" ? "working" : "idle", verb_since_unix_ms: now - spec.ago, request: null, later_by: null, pull_requests: pulls.map((one) => one.pr) },
  } as AgentRow;
}

const card = (root: string, state: TaskCardScope["state"], members: string[], checkouts: string[], recency: string, stage?: TaskCardScope["stage"]): TaskCardScope => ({ root, state, members, checkouts, recency, ...(stage ? { stage } : {}) });

function scopeWith(cards: TaskCardScope[]): AgentScope {
  const needs = cards.filter((one) => one.state === "needs_you" || one.state === "blocked").length;
  const members = cards.flatMap((one) => one.members.map((pane_id) => ({ pane_id, project_id: one.checkouts[0]!.split(":")[0]!, checkout_id: one.checkouts[0]! })));
  return { ...EMPTY_GALLERY_SCOPE, members, total: members.length, overview_total: members.length, tasks: { cards, needs_you: needs, recency: cards[0]?.recency ?? "" } };
}

function issue(repository: string, number: number, title: string, ago: number, now: number, labels: string[] = []): Task {
  const palette: Record<string, string> = { bug: "d73a4a", ux: "a2eeef", perf: "fbca04", docs: "0075ca" };
  return {
    key: `github:${repository}#${number}`, source: "github", id: `#${number}`, url: `https://example.invalid/${repository}/issues/${number}`, title, open: true,
    created_at_unix_ms: now - ago, updated_at_unix_ms: now - ago, labels: labels.map((name) => ({ name, color: palette[name] ?? "cccccc" })),
  };
}

function memo(path: string, number: number, title: string, ago: number, now: number): Task {
  return { key: `local:${path}#${number}`, source: "local", id: `L-${number}`, url: null, title, open: true, created_at_unix_ms: now - ago, updated_at_unix_ms: now - ago };
}

const github = (name: string, isPublic: boolean | undefined, now: number): TaskSource => ({ kind: "github", label: "GitHub", name, reading: false, failure: null, last_read_at_unix_ms: now - 2 * MINUTE, ...(isPublic === undefined ? {} : { public: isPublic }) });
const local = (now: number): TaskSource => ({ kind: "local", label: "Local", name: null, reading: false, failure: null, last_read_at_unix_ms: now - MINUTE });

/** A project's measured disk on the shared, nearly full volume. */
const VOLUME_TOTAL = 494 * GB;
const VOLUME_FREE = 7.4 * GB;
function disk(bytes: number, buildCache: number, dependencies: number): ProjectDisk {
  return {
    total_bytes: bytes, unavailable_reason: null, measuring: false, queued: false, free_bytes: VOLUME_FREE, volume_total_bytes: VOLUME_TOTAL, confirmed_bytes: bytes,
    layers: { build_cache: buildCache, dependencies, source: bytes * 0.08, other: bytes * 0.04, shared_git: bytes * 0.06 },
  };
}

/** A merged worktree's layers, which a cleanup would empty. */
function mergedDisk(path: string, buildCache: number, dependencies: number, now: number): WorktreeDisk {
  const cell = (bytes: number, name: string) => ({ bytes, folders: 3, largest_name: name });
  return { path, total_bytes: buildCache + dependencies + 40 * MB, unavailable_reason: null, measured_at_unix_ms: now - 5 * MINUTE, layers: { build_cache: cell(buildCache, "target"), dependencies: cell(dependencies, "node_modules"), other: cell(12 * MB, "dist"), source_bytes: 28 * MB }, volume_free_bytes: VOLUME_FREE };
}

function merged(row: Checkout, number: number, title: string, now: number, buildCache: number, dependencies: number): Checkout {
  return { ...row, pull_request: { url: `https://example.invalid/pull/${number}`, review: null, badge: "merged", is_draft: false, number, title, state: "merged" }, worktree: { ...row.worktree!, merged: true, disk: mergedDisk(row.path, buildCache, dependencies, now) } };
}

export type OverviewWorld = { rest: SnapshotRest; agents: AgentRow[]; record: SessionRecord; recordSession: WorkSession; recordPane: string; linkSummaries: LinkSummaries };

export function overviewWorld(now: number): OverviewWorld {
  const s = Math.floor(now / 1000);
  const hidePrs = {
    sidebar: pull("example/hide", 140, "Sidebar readable at narrow widths", "pending"),
    release: pull("example/hide", 149, "Release checklist before deploy", "mergeable"),
    search: pull("example/hide", 154, "Search palette", "merged"),
  };
  const billingPr = pull("example/billing-api", 88, "Verify webhook signatures", "failed");
  const agents: AgentRow[] = [
    agent({ pane: "h-a3", title: "배포 전 변경 확인", look: "asking", ago: 3 * MINUTE, ask: { verb: "answer", what: "프로덕션 배포 전에 이번 변경 범위를 확인해 주세요", more: 0 }, pulls: [{ pr: hidePrs.release, state: "mergeable" }] }, now),
    agent({ pane: "h-a1", now: "좁은 폭 줄바꿈 고치는 중", title: "사이드바 가독성 개선", look: "working", ago: MINUTE, children: ["h-a1c1", "h-a1c2", "h-a1c3", "h-a1c4"], pulls: [{ pr: hidePrs.sidebar, state: "pending" }] }, now),
    agent({ pane: "h-a1c1", title: "컴포넌트 구조 정리", look: "working", ago: 40_000, kind: "codex", parent: "h-a1", children: ["h-a1g1"] }, now),
    agent({ pane: "h-a1c2", title: "한글 가독성 확인", look: "done", ago: 6 * MINUTE, parent: "h-a1" }, now),
    agent({ pane: "h-a1c3", title: "긴 한국어와 English title wrapping 검토", look: "working", ago: 3 * MINUTE, parent: "h-a1", kind: "codex" }, now),
    agent({ pane: "h-a1c4", title: "키보드 이동 확인", look: "idle", ago: 15 * MINUTE, parent: "h-a1" }, now),
    agent({ pane: "h-a1g1", title: "모바일 표 너비 조정", look: "working", ago: 2 * MINUTE, parent: "h-a1c1" }, now),
    agent({ pane: "h-e1", now: "English 화면 단축키 확인 중", title: "키보드 단축키와 English 화면 연결 검증", look: "working", ago: 4 * MINUTE, kind: "codex" }, now),
    agent({ pane: "h-q2", now: "PR 머지됨", title: "검색 팔레트", look: "idle", ago: 3 * 60 * MINUTE, pulls: [{ pr: hidePrs.search, state: "merged" }] }, now),
    agent({ pane: "h-r1", now: "초안 끝", title: "릴리스 노트 초안", look: "idle", ago: 26 * 60 * MINUTE }, now),
    agent({ pane: "h-r2", now: "변경 목록 정리 끝", title: "의존성 업데이트 검토", look: "idle", ago: 2 * 24 * 60 * MINUTE }, now),
    agent({ pane: "b-1", title: "결제 재시도 로그 정리", look: "asking", approval: true, ago: 9 * MINUTE, kind: "codex", ask: { verb: "approval", what: "마이그레이션을 운영 DB에 실행할까요?", more: 0 } }, now),
    agent({ pane: "b-2", now: "CI 실패 고치는 중", title: "웹훅 서명 검증 추가", look: "working", ago: 2 * MINUTE, pulls: [{ pr: billingPr, state: "failed" }] }, now),
    agent({ pane: "s-1", now: "문구 3안 끝", title: "랜딩 문구 다듬기", look: "done", ago: 12 * MINUTE }, now),
  ];

  const hideCheckouts: Checkout[] = [
    checkout({ id: "hide:main", workspace: "hide", branch: "main", primary: true, age: 600, panes: ["h-r1", "h-r2"], marks: { quiet: 2 } }, s),
    { ...checkout({ id: "hide:sidebar", workspace: "hide", branch: "feat/sidebar-readable", age: 120, panes: ["h-a1", "h-a1c1", "h-a1c2", "h-a1c3", "h-a1c4", "h-a1g1"], marks: { going: 4, done: 1 } }, s), task_key: hideKey(151) },
    checkout({ id: "hide:release", workspace: "hide", branch: "release/checklist", age: 900, panes: ["h-a3"], marks: { turn: 1 }, pr: { number: 149, title: "Release checklist before deploy", badge: "open", is_draft: false, state: "mergeable" } }, s),
    { ...checkout({ id: "hide:shortcuts", workspace: "hide", branch: "feat/shortcut-settings", age: 300, panes: ["h-e1"], marks: { going: 1 } }, s), task_key: hideKey(158) },
    merged(checkout({ id: "hide:search", workspace: "hide", branch: "feat/search-palette", age: 3 * 3600, panes: ["h-q2"], marks: { quiet: 1 } }, s), 154, "Search palette", now, 1.6 * GB, 640 * MB),
    merged(checkout({ id: "hide:docs", workspace: "hide", branch: "docs/install-guide", age: 2 * 86400 }, s), 147, "Install guide", now, 380 * MB, 210 * MB),
  ];
  const hideTasks = [
    issue("example/hide", 163, "탭을 닫을 때 마지막 뷰 위치 기억하기", 40 * MINUTE, now, ["ux"]),
    issue("example/hide", 161, "큰 저장소에서 파일 트리 첫 표시가 느림", 5 * 60 * MINUTE, now, ["perf"]),
    issue("example/hide", 160, "사용량 팝오버의 날짜 범위 표기 통일", 26 * 60 * MINUTE, now, ["ux"]),
    issue("example/hide", 159, "설치 안내에 Linux 패키지 추가", 3 * 24 * 60 * MINUTE, now, ["docs"]),
    issue("example/hide", 158, "단축키 설정 화면 연결", 4 * 24 * 60 * MINUTE, now),
    issue("example/hide", 151, "좁은 사이드바에서 긴 제목 말줄임", 5 * 24 * 60 * MINUTE, now, ["ux", "bug"]),
    memo("/work/hide", 1, "메모와 GitHub 이슈를 한 프로젝트에서 함께 비교하기", 10 * MINUTE, now),
    { ...memo("/work/hide", 2, "이미 완료한 접근성 검토", 12 * 60 * MINUTE, now), open: false },
  ];
  const billingTasks = [
    issue("example/billing-api", 42, "환불 웹훅이 두 번 오는 경우 처리", 2 * 60 * MINUTE, now, ["bug"]),
    issue("example/billing-api", 39, "결제 실패 사유 코드 정리", 2 * 24 * 60 * MINUTE, now),
  ];
  const sitePath = "/work/site";
  const siteTasks = [
    memo(sitePath, 4, "요금제 표에 연간 할인 표시", 20 * MINUTE, now),
    memo(sitePath, 3, "모바일에서 히어로 이미지 잘림 확인", 3 * 60 * MINUTE, now),
    memo(sitePath, 2, "랜딩 문구 다듬기", 2 * 24 * 60 * MINUTE, now),
  ];

  const recency = (ago: number) => new Date(now - ago).toISOString();
  const hide = workspace("hide", {
    checkouts: hideCheckouts,
    agent_scope: scopeWith([
      card("h-a3", "needs_you", ["h-a3"], ["hide:release"], recency(3 * MINUTE), 3),
      card("h-a1", "working", ["h-a1", "h-a1c1", "h-a1c2", "h-a1c3", "h-a1c4", "h-a1g1"], ["hide:sidebar"], recency(MINUTE), 2),
      card("h-e1", "working", ["h-e1"], ["hide:shortcuts"], recency(4 * MINUTE), 1),
      card("h-q2", "cleanup", ["h-q2"], ["hide:search"], recency(3 * 60 * MINUTE), 4),
      card("h-r1", "resting", ["h-r1"], ["hide:main"], recency(26 * 60 * MINUTE)),
      card("h-r2", "resting", ["h-r2"], ["hide:main"], recency(2 * 24 * 60 * MINUTE)),
    ]),
    tasks: { source: github("example/hide", true, now), sources: [github("example/hide", true, now), local(now)], tasks: hideTasks.map((row, i) => ({ ...row, author: i % 2 ? "mira" : "jules", assignees: i % 3 ? ["jules"] : [] })), overflow: false },
    disk: disk(9.6 * GB, 3.1 * GB, 2.4 * GB),
    last_activity_unix_ms: now - MINUTE,
    agent_presence: { working: true },
  }, REFERENCE_FOLDS);
  const billing = workspace("billing-api", {
    checkouts: [
      checkout({ id: "billing-api:main", workspace: "billing-api", branch: "main", primary: true, age: 1200, panes: ["b-1"], marks: { turn: 1 } }, s),
      checkout({ id: "billing-api:webhook", workspace: "billing-api", branch: "feat/webhook-signature", age: 200, panes: ["b-2"], marks: { going: 1 } }, s),
    ],
    agent_scope: scopeWith([
      card("b-1", "needs_you", ["b-1"], ["billing-api:main"], recency(9 * MINUTE)),
      card("b-2", "working", ["b-2"], ["billing-api:webhook"], recency(2 * MINUTE), 2),
    ]),
    tasks: { source: github("example/billing-api", false, now), tasks: billingTasks, overflow: false },
    disk: disk(2.3 * GB, 0.9 * GB, 0.6 * GB),
    last_activity_unix_ms: now - 2 * MINUTE,
    agent_presence: { working: true },
  }, REFERENCE_FOLDS);
  const site = workspace("site", {
    checkouts: [{ ...checkout({ id: "site:main", workspace: "site", branch: "main", primary: true, age: 900, panes: ["s-1"], marks: { done: 1 } }, s), task_key: `local:${sitePath}#2` }],
    agent_scope: scopeWith([card("s-1", "done", ["s-1"], ["site:main"], recency(12 * MINUTE), 1)]),
    tasks: { source: local(now), tasks: siteTasks, overflow: false },
    disk: disk(640 * MB, 210 * MB, 300 * MB),
    last_activity_unix_ms: now - 12 * MINUTE,
    agent_presence: { working: false },
  }, REFERENCE_FOLDS);
  const notes = workspace("team-notes", {
    is_git: false,
    checkouts: [checkout({ id: "team-notes:folder", workspace: "team-notes", branch: null, folder: true }, s)],
    tasks: { source: local(now), tasks: [], overflow: false },
    disk: disk(18 * MB, 0, 0),
    last_activity_unix_ms: now - 6 * 24 * 60 * MINUTE,
    agent_presence: { working: false },
  }, REFERENCE_FOLDS);
  const workspaces: Workspace[] = [hide, billing, site, notes];
  // A pane head names its agent the way the core labels the pane.
  const titles = new Map(agents.map((row) => [row.pane_id, row.identity_label]));
  for (const project of workspaces) for (const row of project.checkouts) for (const tab of row.tabs) for (const pane of tab.panes) pane.identity_label = titles.get(pane.id) ?? pane.identity_label;

  const recordSession: WorkSession = { device_id: "local", agent: "claude", session_id: "session-h-a1" };
  const backlog: BacklogItem[] = [
    { intent: "scene-creating", workspace_id: "hide", title: "온보딩 첫 화면의 버튼 순서 정리", phase: "creating", task_key: null, failure: null, retry: false, missed: null, at_unix_ms: now - 4_000 },
  ];
  const issueWork: IssueWork = {
    backlog,
    missed_keys: [{ task_key: hideKey(161), device_id: null }],
    made_keys: [hideKey(161), hideKey(163)],
    detail: null,
    name: null,
  };

  const rest: SnapshotRest = {
    overview_graph: {
      available: true, limit: 2000, retained_count: 12, truncated: false, retention_ms: 86_400_000,
      sessions: agents.map((row) => ({ device_id: "local", pane_id: row.pane_id, session_digest: `fixture-${row.pane_id}`, launch_model: row.agent_kind === "codex" ? "model-c" : "model-a" })),
      letters: Array.from({ length: 12 }, (_, i) => ({ id: `fixture-letter-${i}`, sender: { device_id: "local", pane_id: i % 2 ? "h-a1c1" : "b-1", session: i % 2 ? "fixture-h-a1c1" : "fixture-b-1" }, recipient: { device_id: "local", pane_id: i % 2 ? "h-a1" : "h-a3", session: i % 2 ? "fixture-h-a1" : "fixture-h-a3" }, kind: i % 3 ? "progress" : "request", at_unix_ms: now - i * MINUTE })),
      // Herdr input with no letter between two teams draws the dotted direct-only line.
      direct_inputs: Array.from({ length: 3 }, (_, i) => ({ id: `fixture-direct-${i}`, sender: { device_id: "local", pane_id: "h-e1", session: "fixture-h-e1" }, recipient: { device_id: "local", pane_id: "s-1", session: "fixture-s-1" }, at_unix_ms: now - (i + 2) * MINUTE })),
    },
    navigator: {
      workspaces,
      inactive_projects: [],
      no_agent_projects: [],
      sleeping_sessions: [],
      agents,
      devices: [{ agent_scope: EMPTY_GALLERY_SCOPE, id: "local", label: "This Mac", kind: "local", state: "local", message: null, ssh_alias: null, agent_count: agents.length, test: null }],
      focused_device_id: "local",
      focused_checkout_id: null,
    },
    ui_state: {
      left_sidebar_visible: false,
      collapsed_workspace_ids: [],
      session_collapsed_checkout_ids: [],
      workspace_registrations: workspaces.map((row) => ({ id: row.id, label: row.label, path: row.path, device_id: "local", pinned: row.pinned })),
    },
    issue_work: issueWork,
    // Summaries on, so a record draws its flows and summaries rather than the plain request list.
    status: { background_ai: { provider: "claude", providers: [], unavailable_reason: null, enabled: true, chosen: true, agent_summary: true } },
  } as SnapshotRest;

  const request = (offset: number, text: string, ago: number, summary: string | null = null) => ({ offset, text, at_unix_ms: now - ago, author: "operator" as const, images: 0, summary, answers: null, letter_from: null, count: 1, goal_command: false });
  const blank = { asks: [], omitted_requests: 0, outputs: [], pr_groups: [], artifacts: [], children: [] };
  const child = (pane: string, title: string): SessionRecord => ({
    ...blank,
    session: { device_id: "local", agent: pane === "h-a1c1" ? "codex" : "claude", session_id: `session-${pane}` }, title, fallback_title: "Agent", native_title: title, manual_title: null, state: pane === "h-a1c1" ? "working" : "idle",
    status: pane === "h-a1c1" ? null : { end: "done", line: `${title} 끝`, at_unix_ms: now - 10 * MINUTE, limited: null, settled: null },
    cwd: "/work/hide.worktrees/feat/sidebar-readable", path: null, started_at_unix_ms: now - 40 * MINUTE, ended_at_unix_ms: null,
    flows: [{ id: "c", name: title, at_unix_ms: now - 40 * MINUTE, requests: [], request_count: 0, result: null }],
  });
  const output = { kind: "pr" as const, repository: "example/hide", number: 140, title: "Sidebar readable at narrow widths", url: "https://example.invalid/example/hide/pull/140", state: "review", stale: false, last_read_at_unix_ms: now - MINUTE, created_at_unix_ms: now - 10 * MINUTE, source: recordSession, source_name: "사이드바 가독성 개선" };
  const record: SessionRecord = {
    ...blank,
    session: recordSession, title: "사이드바 가독성 개선", fallback_title: "Claude", native_title: "사이드바 가독성 개선", manual_title: null, state: "working",
    cwd: "/work/hide.worktrees/feat/sidebar-readable", path: null, started_at_unix_ms: now - 70 * MINUTE, ended_at_unix_ms: null,
    status: { end: "working", line: "한글 가독성 확인 중", at_unix_ms: now - 4 * MINUTE, limited: null, settled: null },
    asks: [{ id: "ask-1", kind: "ask", text: "시간 열을 좁은 폭에서 숨길지", status: "open", at_unix_ms: now - 6 * MINUTE, turn_offset: 2, issue_repository: null, issue_number: null, backlog_key: null, creating: false }],
    flows: [{
      id: "chapter-1", name: "좁은 사이드바에서도 긴 제목이 읽히게", at_unix_ms: now - 70 * MINUTE, result: null,
      requests: [
        request(1, `${hideRef(151)} 좁은 사이드바에서 긴 제목이 잘리는 문제부터 고쳐줘. 툴팁으로 전체 제목이 보이게`, 70 * MINUTE, "긴 제목 말줄임과 툴팁으로 전체 제목 보이기"),
        request(2, "컴포넌트 구조 정리는 자식 에이전트 하나 띄워서 맡기고, 너는 한글 가독성 확인해줘", 35 * MINUTE, "구조 정리는 자식에게 맡기고 한글 가독성 확인"),
        request(3, "PR 열면 체크 통과하는지 보고 알려줘", 4 * MINUTE, "PR 체크 통과 여부 알려주기"),
        ...Array.from({ length: 6 }, (_, index) => request(4 + index, `후속 요청 ${index + 1}: 확인 항목을 기록해줘`, (3 - index / 3) * MINUTE)),
      ],
      request_count: 9,
    }],
    outputs: [output],
    pr_groups: [{ child: null, name: "사이드바 가독성 개선", outputs: [output] }],
    artifacts: ["사이드바 가독성 점검표"],
    children: [child("h-a1c1", "컴포넌트 구조 정리"), child("h-a1c2", "한글 가독성 확인")],
  };
  const linkSummaries: LinkSummaries = { projects: {}, filling: false };
  return { rest, agents, record, recordSession, recordPane: "h-a1", linkSummaries };
}

/** Focus fixtures change only this gallery route; other Overview tabs keep their reference world. */
export function overviewFocusWorld(now: number, content: string): OverviewWorld {
  const world = overviewWorld(now);
  const agents = world.agents;
  const get = (pane: string) => agents.find((row) => row.pane_id === pane)!;
  Object.assign(get("h-a1"), { mark: "quiet", group: "seen", activity: "idle", state: { ...get("h-r1").state } });
  Object.assign(get("h-a3"), { group: "seen", unread: false, state: { ...get("h-a3").state, needs_you: false, attention: false, mark_tone: { kind: "warning", read: true } } });
  Object.assign(get("h-q2"), { mark: "unknown", identity_label: "상태 확인 중 · unknown" });
  Object.assign(get("h-r2"), { mark: "asleep", identity_label: "잠든 릴리스 세션", lineage_parent_pane_id: "h-r1" });
  if (content === "long") for (const row of agents) row.identity_label += " · 긴 한국어 제목과 English identifiers 줄바꿈 확인";
  const extraProjects: Workspace[] = [];
  const sizes = content === "many" || content === "dense" ? [48, 30, ...Array(19).fill(8)] : content === "large" ? [48] : [3, 3, 3, 3, 2];
  for (const [index, count] of sizes.entries()) {
    const id = `focus-project-${index}`;
    const panes = Array.from({ length: count }, (_, session) => `focus-pane-${index}-${session}`);
    const rows = panes.map((pane, session) => agent({ pane, title: `실험 작업 ${index + 1} · ${session + 1}${content === "long" ? " · 긴 한국어 제목과 English identifiers" : ""}`, look: session % 3 === 0 ? "working" : "idle", ago: MINUTE, ...(session > 0 && session % 3 === 0 ? { parent: panes[0] } : {}) }, now));
    agents.push(...rows);
    const one = checkout({ id: `${id}:main`, workspace: id, branch: "main", primary: true, panes }, Math.floor(now / 1000));
    extraProjects.push(workspace(id, { label: `실험 프로젝트 ${index + 1}${content === "long" ? " · Korean and English 프로젝트 이름 줄바꿈" : ""}`, checkouts: [one], agent_scope: scopeWith(rows.map((row) => card(row.pane_id, "working", [row.pane_id], [one.id], "0"))) }, REFERENCE_FOLDS));
  }
  world.rest.navigator!.workspaces!.push(...extraProjects);
  if (content === "connection") for (const [index, row] of agents.entries()) {
    row.identity_label += " · 긴 한국어와 English 저장 대화 제목";
    row.record_connection = (["checking", "unconfirmed", "confirmed"] as const)[index % 3];
    row.record_summary = "기록에 보관한 진행 요약 · saved conversation summary";
  }
  if (content === "resting") for (const row of agents) Object.assign(row, { mark: "quiet", group: "seen", state: { ...get("h-r1").state } });
  if (content === "direct") world.rest.overview_graph!.direct_inputs = [
    { id: "fixture-direct-one", sender: { device_id: "local", pane_id: "h-a1c1", session: "fixture-h-a1c1" }, recipient: { device_id: "local", pane_id: "h-a1", session: "fixture-h-a1" }, at_unix_ms: now - MINUTE },
    { id: "fixture-direct-two", sender: { device_id: "local", pane_id: "h-a1c1", session: "fixture-h-a1c1" }, recipient: { device_id: "local", pane_id: "h-a1", session: "fixture-h-a1" }, at_unix_ms: now - 2 * MINUTE },
    { id: "fixture-direct-cross-project", sender: { device_id: "local", pane_id: "b-1", session: "fixture-b-1" }, recipient: { device_id: "local", pane_id: "h-a3", session: "fixture-h-a3" }, at_unix_ms: now - 3 * MINUTE },
  ];
  if (content === "empty") {
    agents.splice(0);
    for (const project of world.rest.navigator!.workspaces!) { project.agent_scope = scopeWith([]); for (const checkout of project.checkouts) checkout.agent_scope = scopeWith([]); }
  }
  world.rest.overview_graph!.sessions = agents.map((row) => ({ device_id: "local", pane_id: row.pane_id, session_digest: `fixture-${row.pane_id}`, launch_model: row.agent_kind === "codex" ? "model-c" : "model-a" }));
  return world;
}
