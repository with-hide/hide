import statusAxisScope from "./sidebarStatusAxisScope.json";
import type { AgentScope } from "../agentScope";
import { EMPTY_GALLERY_SCOPE, galleryScopes } from "./agentScopes";
// The Projects sidebar's gallery scene (PRD design-review-workflow D-04,
// B3, B4): a synthetic snapshot the real sidebar renders, with the content
// `Screen / Projects Sidebar` in design/hide-screens.pen draws, so an actual
// capture and the Pen frame show the same projects, checkouts and agents.
// Everything here is invented example data: no path, name or count comes from
// a machine. The folds are this scene's own ui state, applied by `applyEvent`
// the way the core applies the same events, so a chevron really folds.

import type { AgentRow, Checkout, MarkCounts, PullRequest, SleepingSession, SnapshotRest, Workspace } from "../snapshot";
import { galleryAgentState } from "./agentStates";

/** Which titles the scene carries: the Pen frame's, or long Korean ones for truncation. */
export type SceneContent = "reference" | "long";

/**
 * How many devices the scene registers: This Mac alone (the rail still
 * shows, with one tile), This Mac, a connected `mini` and a disconnected one
 * with a long Korean name (quick device-rail-badges), or that fixture with
 * `mini` busy enough to draw the `9+` pill.
 */
export type SceneDevices = "one" | "two" | "busy";

/** The device the scene opens in front: a rail tile's id. */
export type SceneFront = "local" | "mini" | "offline";

/** The registered-but-unreachable device of the two-device scene. */
export const OFFLINE_DEVICE = "build-box";

/** The fold choices the core would keep in its ui state. */
export type SceneFolds = {
  collapsedCheckouts: string[];
  collapsedWorkspaces: string[];
  /** Parents whose descendants are unfolded; every other parent is folded, the core's default. */
  expandedAgents: string[];
  inactiveCheckoutsOpen: string[];
  inactiveProjectsOpen: boolean;
  /** The No agents fold is open (`no_agents/local`, PRD sidebar-titlebar-refresh D-01). */
  noAgentsOpen: boolean;
  /** Sleeping projects the operator opened (`asleep/<project>`, D-19); every other one starts folded. */
  asleepOpen: string[];
  /** The device the core has in front (`focus_device`); a rail tile changes it. */
  frontDevice: string;
};

/** The folds `Screen / Projects Sidebar` draws: main's agents open, a1 unfolded, sasu folded. */
export const REFERENCE_FOLDS: SceneFolds = {
  collapsedCheckouts: [],
  collapsedWorkspaces: ["sasu"],
  expandedAgents: ["a1"],
  inactiveCheckoutsOpen: [],
  inactiveProjectsOpen: false,
  noAgentsOpen: false,
  asleepOpen: [],
  frontDevice: "local",
};

/** The projects with no agent row and no sleeping conversation, which the core folds under No agents. */
const NO_AGENT_PROJECTS = ["investment", "landing-page", "modakbul-site"];
/** The projects whose every agent sleeps, with how many days ago the latest fell asleep. */
const SLEEPING_PROJECTS: Record<string, number> = { "codex-daemon-apply-spec": 4, "sticky-notes": 14 };

const ROOT = "/work";

const TITLES: Record<SceneContent, Record<string, string>> = {
  reference: {
    a1: "사이드바 가독성 개선",
    a1c1: "컴포넌트 구조 정리",
    a1c2: "한글 가독성 확인",
    a2: "후속 UX 계획 인터뷰",
    a3: "배포 전 확인",
  },
  long: {
    a1: "사이드바 자식 에이전트 들여쓰기와 긴 한글 제목의 말줄임을 함께 확인하는 작업",
    a1c1: "컴포넌트 구조 정리와 상태 열 정렬을 위한 하위 작업",
    a1c2: "한글 가독성 확인 및 좁은 폭에서 시간 열이 밀리지 않는지 검토",
    a2: "후속 UX 계획 인터뷰 결과를 정리해서 다음 단계 제안서로 만들기",
    a3: "배포 전 확인 요청에 대한 답변 대기",
  },
};

/** A scene names how long ago each agent changed (`42s`, `3m`, `2h`); the row counts on from there. */
type AgentSpec = Partial<AgentRow> & Pick<AgentRow, "pane_id" | "group" | "mark" | "status_code"> & { elapsed: string };

const UNIT_MS: Record<string, number> = { s: 1_000, m: 60_000, h: 3_600_000, d: 86_400_000 };

function agent({ elapsed, ...spec }: AgentSpec): AgentRow {
  const unit = UNIT_MS[elapsed.slice(-1)] ?? 0;
  const changed = Date.now() - Number(elapsed.slice(0, -1)) * unit;
  return {
    state: galleryAgentState(spec.pane_id, spec.detail, changed),
    id: spec.pane_id,
    identity_label: spec.pane_id,
    agent_kind: "claude",
    // The core draws a row bright exactly while it is in Needs You or Done.
    emphasized: spec.group === "needs_you" || spec.group === "done",
    unread: false,
    demand: "none",
    activity: "idle",
    changed_at_unix_ms: changed,
    ...spec,
  };
}

/** A checkout's agents by mark; `quiet` ones count in Seen and on no badge. */
type SpecMarks = Partial<MarkCounts> & { quiet?: number };

function marks(spec: SpecMarks): MarkCounts {
  return { turn: spec.turn ?? 0, going: spec.going ?? 0, done: spec.done ?? 0, unread: spec.unread ?? 0 };
}

type CheckoutSpec = {
  id: string;
  workspace: string;
  branch: string | null;
  primary?: boolean;
  folder?: boolean;
  /** Seconds since the last commit; absent on a plain folder. */
  age?: number;
  purpose?: string;
  panes?: string[];
  marks?: SpecMarks;
  pr?: Pick<PullRequest, "badge" | "is_draft" | "number" | "title" | "state">;
  exists?: boolean;
};

export function checkout(spec: CheckoutSpec, nowSeconds: number): Checkout {
  const base = `${ROOT}/${spec.workspace}`;
  const path = spec.primary || spec.folder ? base : `${base}.worktrees/${spec.branch}`;
  const panes = spec.panes ?? [];
  const counted = spec.marks ? marks(spec.marks) : null;
  return {
    agent_scope: EMPTY_GALLERY_SCOPE,
    id: spec.id,
    workspace_id: spec.workspace,
    label: spec.branch ?? spec.workspace,
    path,
    branch: spec.branch,
    purpose: spec.purpose ? { text: spec.purpose, origin: "operator" } : null,
    is_worktree: !spec.primary && !spec.folder,
    is_primary: spec.primary === true,
    exists: spec.exists ?? true,
    has_panes: panes.length > 0,
    agent_summary: counted
      ? {
          representative_pane_id: panes[0] ?? null,
          needs_you: counted.turn,
          done: counted.done + counted.unread,
          working: counted.going,
          seen: spec.marks?.quiet ?? 0,
          unknown: 0,
          marks: counted,
        }
      : undefined,
    worktree: spec.folder
      ? null
      : {
          path,
          branch: spec.branch,
          head_sha: "0000000",
          last_commit_unix_seconds: spec.age === undefined ? null : nowSeconds - spec.age,
          is_main: Boolean(spec.primary),
          missing: spec.exists === false,
          dirty: false,
          changed_file_count: 0,
          merged: false,
          behind_upstream: null,
          pane_count: panes.length,
          deletion_gate: { blocked_reason: null, warnings: [], button_label: "Delete worktree", can_delete_branch: true, branch_warning: null, discard_label: null },
        },
    pull_request: spec.pr ? { url: "", review: null, ...spec.pr } : null,
    github: { failure_category: null, available: true, loading: false, stale: false, last_success_at_unix_ms: nowSeconds * 1000, unavailable_reason: null },
    tabs: panes.length ? [{ id: `${spec.id}:t1`, workspace_id: spec.workspace, checkout_id: spec.id, label: "1", sidebar: { manual_name: null, number: 1, named_by_operator: false }, empty: false, delegated: false, panes: panes.map(pane) }] : [],
    active_tab_id: panes.length ? `${spec.id}:t1` : null,
    strip: [],
    next_tab_label: "2",
  };
}

function pane(id: string) {
  return {
    id,
    herdr_label: null,
    terminal_title: null,
    cwd: ROOT,
    status_code: "idle" as const,
    requires_close_confirmation: false,
    requires_close_status_check: false,
    identity_label: id,
  };
}

export function workspace(id: string, extra: Partial<Workspace> & Pick<Workspace, "checkouts">, folds: SceneFolds): Workspace {
  return {
    agent_scope: EMPTY_GALLERY_SCOPE,
    label: id,
    path: `${ROOT}/${id}`,
    device_id: "local",
    registered: true,
    temporary: false,
    pinned: false,
    is_git: true,
    expanded: !folds.collapsedWorkspaces.includes(id),
    inactive_checkouts: { expanded: folds.inactiveCheckoutsOpen.includes(`${ROOT}/${id}`), checkout_ids: [] },
    ...extra,
    id,
  };
}

/**
 * What the core says of each project's agents (`agent_presence`): working
 * while a checkout counts a mark other than the resting ring, asleep for the
 * scene's sleeping projects, which follow their own fold.
 */
function withPresence(row: Workspace, folds: SceneFolds, nowMs: number): Workspace {
  const days = SLEEPING_PROJECTS[row.id];
  if (days !== undefined) return { ...row, agent_presence: { working: false, asleep: { since_unix_ms: nowMs - days * 86_400_000, expanded: folds.asleepOpen.includes(row.id) } } };
  const working = row.checkouts.some((value) => {
    const counts = value.agent_summary?.marks;
    return !!counts && counts.turn + counts.going + counts.done + counts.unread > 0;
  });
  return { ...row, agent_presence: { working } };
}

/** a1's children in the order the core lists them; the sidebar ranks them most urgent first. */
const A1_CHILDREN = ["a1c1", "a1c2", "a1c3", "a1c4", "a1c5", "a1c6", "a1c7", "a1c8"];

type PullState = "failed" | "pending" | "mergeable" | "merged";

/** One PR the row holds the duty of (D-39); its summary is the pane's state preset in agentStates.json. */
function ownPull(number: number, title: string, state: PullState): Pick<AgentRow, "request"> {
  return {
    request: {
      verb: "working", verb_since_unix_ms: 0, line: undefined, request: null, later_by: null,
      pull_requests: [{ number, title, url: `https://example.invalid/hide/pull/${number}`, badge: state === "merged" ? "merged" : "open", checks: state === "failed" ? "failed" : state === "pending" ? "pending" : "passing", review: state === "pending" ? "review_required" : "approved", head_branch: `agents/${number}`, closing_issues: [], live: true, duty: true, created: true, settled_at_unix_ms: state === "merged" ? 0 : null }],
    },
  };
}

/** Fourteen settled worktrees behind hide's `Inactive 14`; `feat/ui` is where a1's first child works. */
const INACTIVE = ["feat/ui", ...Array.from({ length: 13 }, (_, index) => `chore/cleanup-${index + 1}`)];

/** The scene's snapshot and agents for one content set and one set of folds. */
export function sidebarScene(content: SceneContent, folds: SceneFolds, nowMs: number, devices: SceneDevices = "one"): { rest: SnapshotRest; agents: AgentRow[] } {
  const now = Math.floor(nowMs / 1000);
  const title = TITLES[content];
  const unfolded = (id: string) => folds.expandedAgents.includes(id);
  const agents: AgentRow[] = [
    agent({ pane_id: "tn1", identity_label: "회의록 요약", group: "seen", mark: "quiet", status_code: "idle", elapsed: "3h" }),
    agent({
      pane_id: "a1",
      identity_label: title.a1,
      group: "working",
      mark: "going",
      status_code: "working",
      activity: "working",
      elapsed: "1m",
      lineage_child_pane_ids: A1_CHILDREN,
      lineage_collapsed: !unfolded("a1"),
      descendant_counts: { error: 0, approval: 1, question: 1, working: 1, done: 1 },
      descendant_mark: { kind: "raised", count: 1 },
      ...ownPull(140, "사이드바 트리 행", "mergeable"),
    }),
    agent({
      pane_id: "a1c1",
      identity_label: title.a1c1,
      agent_kind: "codex",
      group: "working",
      mark: "going",
      status_code: "working",
      activity: "working",
      elapsed: "42s",
      delegated: true,
      lineage_parent_pane_id: "a1",
      lineage_depth: 1,
      lineage_worktree_badge: "feat/ui",
    }),
    agent({ pane_id: "a1c2", identity_label: title.a1c2, group: "seen", mark: "quiet", status_code: "idle", elapsed: "38s", delegated: true, lineage_parent_pane_id: "a1", lineage_depth: 1 }),
    agent({
      pane_id: "a2",
      identity_label: title.a2,
      group: "working",
      mark: "quiet",
      status_code: "done",
      activity: "stopped",
      elapsed: "2m",
      wait: "children",
      lineage_child_pane_ids: ["a2c1", "a2c2"],
      lineage_collapsed: !unfolded("a2"),
      descendant_counts: { error: 0, approval: 0, question: 1, working: 1, done: 0 },
    }),
    agent({
      pane_id: "a2c1",
      identity_label: "인터뷰 질문 정리",
      group: "working",
      mark: "turn",
      status_code: "question",
      demand: "question",
      elapsed: "1m",
      delegated: true,
      detail: "질문 목록을 이대로 보내도 될까요?",
      lineage_parent_pane_id: "a2",
      lineage_depth: 1,
    }),
    agent({ pane_id: "a2c2", identity_label: "사례 조사", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "50s", delegated: true, lineage_parent_pane_id: "a2", lineage_depth: 1 }),
    agent({
      pane_id: "a3",
      identity_label: title.a3,
      group: "needs_you",
      mark: "turn",
      status_code: "question",
      demand: "question",
      elapsed: "30s",
      unread: true,
      detail: "프로덕션 배포 전에 변경 내용을 확인해 주세요",
    }),
    agent({ pane_id: "q1", identity_label: "브라우저 표시 확인", group: "needs_you", mark: "turn", status_code: "question", demand: "question", elapsed: "40m", detail: "주소 경계를 어디에 둘까요?" }),
    agent({ pane_id: "q2", identity_label: "검색 팔레트", group: "done", mark: "done", status_code: "done", activity: "stopped", elapsed: "1h" }),
    agent({ pane_id: "e1", identity_label: "단축키 연결", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "2h" }),
  ];

  if (devices !== "one") agents.push(...HOME_AGENTS[content]);
  // The rest of a1's tree the sheet draws (PRD agent-hierarchy-screens B11, B12, B22): a child waiting
  // on a raised grandchild, an asking child, PR icons in each state, and three behind `3개 더`. They come
  // after every listed row so the scope fixture's list indices stay put.
  agents.push(
    agent({
      pane_id: "a1c3",
      identity_label: "P8 읽기 전용 준비",
      agent_kind: "codex",
      group: "working",
      mark: "quiet",
      status_code: "waiting",
      activity: "stopped",
      elapsed: "19h",
      delegated: true,
      wait: "children",
      lineage_parent_pane_id: "a1",
      lineage_depth: 1,
      lineage_child_pane_ids: ["a1g1", "a1g2"],
      lineage_collapsed: false,
      descendant_mark: { kind: "raised", count: 1 },
      ...ownPull(811, "읽기 전용 준비", "failed"),
    }),
    agent({ pane_id: "a1g1", identity_label: "테스트 작성", agent_kind: "codex", group: "working", mark: "going", status_code: "approval", demand: "approval", elapsed: "12m", delegated: true, lineage_parent_pane_id: "a1c3", lineage_depth: 2 }),
    agent({ pane_id: "a1g2", identity_label: "문서 정리", agent_kind: "codex", group: "seen", mark: "quiet", status_code: "done", activity: "stopped", elapsed: "40m", delegated: true, lineage_parent_pane_id: "a1c3", lineage_depth: 2 }),
    agent({ pane_id: "a1c4", identity_label: "P7 Pi·omp 재우기", group: "working", mark: "going", status_code: "question", demand: "question", elapsed: "1h", delegated: true, lineage_parent_pane_id: "a1", lineage_depth: 1, ...ownPull(838, "Pi·omp 재우기", "pending") }),
    agent({ pane_id: "a1c5", identity_label: "미전달 편지 승격 해제", group: "seen", mark: "quiet", status_code: "idle", elapsed: "5h", delegated: true, lineage_parent_pane_id: "a1", lineage_depth: 1 }),
    agent({ pane_id: "a1c6", identity_label: "P6 OpenCode 세션 리더", group: "seen", mark: "quiet", status_code: "done", activity: "stopped", elapsed: "7m", delegated: true, lineage_parent_pane_id: "a1", lineage_depth: 1, ...ownPull(842, "OpenCode 세션 리더", "mergeable") }),
    agent({ pane_id: "a1c7", identity_label: "P5 큰 도구 결과 읽기", group: "seen", mark: "quiet", status_code: "idle", elapsed: "1h", delegated: true, lineage_parent_pane_id: "a1", lineage_depth: 1, ...ownPull(830, "큰 도구 결과 읽기", "merged") }),
    agent({ pane_id: "a1c8", identity_label: "문서 링크 점검", group: "seen", mark: "quiet", status_code: "idle", elapsed: "3h", delegated: true, lineage_parent_pane_id: "a1", lineage_depth: 1 }),
  );

  const herdrCheckouts: Checkout[] = [
    checkout({ id: "hide:main", workspace: "hide", branch: "main", primary: true, age: 10, purpose: "사이드바 가독성 개선", panes: ["a1", ...A1_CHILDREN.filter((pane) => pane !== "a1c1"), "a1g1", "a1g2", "a2", "a2c1", "a2c2", "a3"], marks: { turn: 2, going: 3, quiet: 1 } }, now),
    checkout({ id: "hide:155", workspace: "hide", branch: "quick/155-browser-display", age: 40 * 60, purpose: issueTitle(155, "browser display (WebContentsView)"), panes: ["q1"], marks: { turn: 1 } }, now),
    checkout({ id: "hide:154", workspace: "hide", branch: "quick/154-search-palette", age: 3600, purpose: issueTitle(154, "⌘K search palette UI"), panes: ["q2"], marks: { done: 1 }, pr: { number: 154, title: "Search palette", badge: "open", is_draft: true, state: "draft" } }, now),
    checkout({ id: "hide:electron", workspace: "hide", branch: "electron-shortcut-bindings", age: 2 * 3600, purpose: "Electron desktop host for the web shell", panes: ["e1"], marks: { going: 1 }, pr: { number: 149, title: "Electron host", badge: "open", is_draft: false, state: "pending" } }, now),
    checkout({ id: "hide:ux", workspace: "hide", branch: "design/workspace-ux-proposal", age: 5 * 3600, purpose: "Workspace UX 제안과 상태 소유 정리" }, now),
    checkout({ id: "hide:fix", workspace: "hide", branch: "fix/registered-projects-only", age: 86400 }, now),
    checkout({ id: "hide:legacy", workspace: "hide", branch: "legacy-shell", exists: false }, now),
    ...INACTIVE.map((branch, index) =>
      checkout({ id: `hide:inactive-${index}`, workspace: "hide", branch, age: (8 + index) * 86400, ...(index === 0 ? { panes: ["a1c1"], marks: { going: 1 } } : {}) }, now),
    ),
  ];

  const workspaces: Workspace[] = [
    workspace("team-notes", { is_git: false, pinned: true, checkouts: [checkout({ id: "team-notes:folder", workspace: "team-notes", branch: null, folder: true, purpose: "회의록 요약 정리", panes: ["tn1"], marks: { quiet: 1 } }, now)] }, folds),
    workspace(
      "hide",
      {
        checkouts: herdrCheckouts,
        inactive_checkouts: { expanded: folds.inactiveCheckoutsOpen.includes(`${ROOT}/hide`), checkout_ids: INACTIVE.map((_, index) => `hide:inactive-${index}`) },
      },
      folds,
    ),
    workspace("sasu", { checkouts: [checkout({ id: "sasu:main", workspace: "sasu", branch: "main", primary: true, age: 3 * 3600, purpose: "파이프라인 정리", marks: { going: 1, done: 1, quiet: 1 } }, now)] }, folds),
    workspace("codex-daemon-apply-spec", { is_git: false, checkouts: [checkout({ id: "codex-daemon-apply-spec:folder", workspace: "codex-daemon-apply-spec", branch: null, folder: true, age: 4 * 86400 }, now)] }, folds),
    workspace("sticky-notes", { checkouts: [checkout({ id: "sticky-notes:main", workspace: "sticky-notes", branch: "main", primary: true, age: 27 * 86400 }, now)] }, folds),
    ...NO_AGENT_PROJECTS.map((id, index) => workspace(id, { checkouts: [checkout({ id: `${id}:main`, workspace: id, branch: "main", primary: true, age: (2 + index) * 86400 }, now)] }, folds)),
    ...["old-prototype", "dotfiles", "research-notes"].map((id, index) =>
      workspace(id, { checkouts: [checkout({ id: `${id}:main`, workspace: id, branch: "main", primary: true, age: (30 + index) * 86400 }, now)] }, folds),
    ),
  ].map((row) => withPresence(row, folds, nowMs));
  if (content === "long") for (const project of workspaces) for (const checkout of project.checkouts) for (const tab of checkout.tabs) {
    if (tab.sidebar) Object.assign(tab.sidebar, { manual_name: "아주 긴 한국어 탭 이름과 작업 묶음의 경계 확인", named_by_operator: true });
  }
  const sleeping: SleepingSession[] = Object.entries(SLEEPING_PROJECTS).map(([id, days]) => ({
    sleep_id: `sleep-${id}`,
    node_id: "local",
    checkout_path: workspaces.find((row) => row.id === id)!.checkouts[0]!.path,
    kind: "claude",
    identity_label: id === "sticky-notes" ? "메모 동기화 정리" : "codex-daemon 적용 이슈 #64 정리",
    group: "idle",
    phase: "sleeping",
    since_unix_ms: nowMs - days * 86_400_000,
    reason: null,
    wake_available: true,
    checking: false,
  }));

  const world = devices !== "one" ? deviceWorld(content, workspaces, now, folds, devices === "busy") : null;
  const rest: SnapshotRest = {
    navigator: {
      workspaces: world ? [...workspaces, world.home] : workspaces,
      inactive_projects: [{ device_id: "local", expanded: folds.inactiveProjectsOpen, project_ids: ["old-prototype", "dotfiles", "research-notes"] }],
      no_agent_projects: [{ device_id: "local", expanded: folds.noAgentsOpen, project_ids: NO_AGENT_PROJECTS }],
      sleeping_sessions: sleeping,
      agents,
      devices: [{ agent_scope: EMPTY_GALLERY_SCOPE, id: "local", label: "This Mac", kind: "local", state: "local", message: null, ssh_alias: null, agent_count: agents.length, test: null }, ...(world?.devices ?? [])],
      focused_device_id: folds.frontDevice,
      focused_checkout_id: null,
    },
    ui_state: {
      left_sidebar_visible: true,
      collapsed_workspace_ids: folds.collapsedWorkspaces,
      session_collapsed_checkout_ids: folds.collapsedCheckouts,
      workspace_registrations: world?.registrations ?? workspaces.map((row) => registrationOf(row, "local")),
    },
    ...(world ? { status: { remote: world.remote } } : {}),
  };
  return { rest: galleryScopes(rest, `${content}:${devices}`, folds.expandedAgents), agents };
}

/** The approved 2-1-2 cutout, rendered by the production Sidebar with invented facts. */
export function sidebarStatusAxisScene(content: SceneContent, folds: SceneFolds, nowMs: number): { rest: SnapshotRest; agents: AgentRow[] } {
  const base = sidebarScene(content, folds, nowMs);
  const labels = ["탭 경계·여백 정리", "왼쪽 pane 검토", "세션 기록 동기화", "PR 질문 표시 개선", "탐색 개선"];
  const ages = ["1m", "8m", "2s", "6m", "11m"];
  const ids = ["a1", "a2", "a3", "q1", "e1"];
  const agents = ids.map((id, index) => agent({ pane_id: id, identity_label: content === "long" ? `${labels[index]} · 여러 프로젝트의 한국어 작업 기록과 진행 상황을 확인합니다` : labels[index], agent_kind: index > 0 && index < 4 ? "codex" : "claude", group: index === 4 ? "seen" : "working", mark: index === 4 ? "quiet" : "going", status_code: index === 4 ? "idle" : "working", elapsed: ages[index]!, state: galleryAgentState(index === 4 ? "h2" : "e1", null, nowMs) }));
  const main = checkout({ id: "hide:main", workspace: "hide", branch: "main", primary: true, age: 480, panes: ids, marks: { going: 4, quiet: 1 } }, nowMs / 1000);
  main.tabs = [["a1", "a2"], ["a3"], ["q1", "e1"]].map((panes, index) => ({ ...main.tabs[0]!, id: `hide:main:t${index + 1}`, sidebar: { number: index + 1, manual_name: index === 0 ? "아주 긴 한국어 탭 이름과 작업 묶음의 경계 확인 · Sidebar tab groups" : null, named_by_operator: index === 0 }, panes: panes.map(pane) }));
  const scope = statusAxisScope as unknown as AgentScope;
  main.agent_scope = scope;
  const project = workspace("hide", { checkouts: [main], agent_scope: scope, agent_presence: { working: true } }, folds);
  const rest = { ...base.rest, navigator: { ...base.rest.navigator, agent_scope: scope, devices: [{ ...base.rest.navigator!.devices![0]!, agent_scope: scope, agent_count: 5 }], workspaces: [project], agents, sleeping_sessions: [], inactive_projects: [], no_agent_projects: [] }, ui_state: { ...base.rest.ui_state, workspace_registrations: [registrationOf(project, "local")] } };
  return { rest, agents };
}

const HOME_AGENTS: Record<SceneContent, AgentRow[]> = {
  reference: [
    agent({ pane_id: "h1", identity_label: "블로그 초안 정리", group: "needs_you", mark: "turn", status_code: "question", demand: "question", elapsed: "2m", unread: true, detail: "톤을 이대로 갈까요?" }),
    agent({ pane_id: "h2", identity_label: "두 프로젝트 비교 조사", agent_kind: "codex", group: "seen", mark: "quiet", status_code: "idle", elapsed: "14m" }),
  ],
  long: [
    agent({ pane_id: "h1", identity_label: "여러 프로젝트에 걸친 블로그 초안 정리와 어투 통일 작업을 이어서 진행하는 에이전트", group: "needs_you", mark: "turn", status_code: "question", demand: "question", elapsed: "2m", unread: true, detail: "톤을 이대로 갈까요, 아니면 조금 더 딱딱하게 다듬을까요?" }),
    agent({ pane_id: "h2", identity_label: "두 프로젝트의 구조와 의존성을 나란히 비교 조사하는 작업", agent_kind: "codex", group: "seen", mark: "quiet", status_code: "idle", elapsed: "14m" }),
  ],
};

const REMOTE_AGENTS: Record<SceneContent, AgentRow[]> = {
  reference: [
    agent({ pane_id: "remote:mini:pane:1", identity_label: "배치 감시", agent_kind: "codex", group: "needs_you", mark: "turn", status_code: "question", demand: "question", elapsed: "5m", unread: true, detail: "풀 리퀘스트 머지할까요?" }),
    agent({ pane_id: "remote:mini:pane:2", identity_label: "릴리스 빌드 확인", agent_kind: "codex", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "1m" }),
  ],
  long: [
    agent({ pane_id: "remote:mini:pane:1", identity_label: "밤새 도는 배치 작업의 실패 원인과 재시도 여부를 감시하는 에이전트", agent_kind: "codex", group: "needs_you", mark: "turn", status_code: "question", demand: "question", elapsed: "5m", unread: true, detail: "풀 리퀘스트 머지할까요?" }),
    agent({ pane_id: "remote:mini:pane:2", identity_label: "릴리스 빌드 확인", agent_kind: "codex", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "1m" }),
  ],
};

/** Enough more agents on `mini` for ten or more Needs You, past the Projects list's cap, an unseen Done and a second Working. */
const BUSY_AGENTS: AgentRow[] = [
  ...Array.from({ length: 11 }, (_, index) =>
    agent({ pane_id: `remote:mini:busy:${index}`, identity_label: `배치 ${index + 1}`, agent_kind: "codex", group: "needs_you", mark: "turn", status_code: "question", demand: "question", elapsed: "3m", detail: "확인이 필요합니다" }),
  ),
  agent({ pane_id: "remote:mini:done:1", identity_label: "빌드 정리", agent_kind: "codex", group: "done", mark: "done", status_code: "done", activity: "stopped", elapsed: "9m" }),
  agent({ pane_id: "remote:mini:working:2", identity_label: "로그 수집", agent_kind: "codex", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "2m" }),
];

/** The registration the core keeps for a project on `deviceId`; `home` marks the device's Home. */
function registrationOf(workspace: Workspace, deviceId: string, extra: { home?: boolean } = {}) {
  return { id: workspace.id, label: workspace.label, path: workspace.path, device_id: deviceId, pinned: workspace.pinned, ...extra };
}

/**
 * The devices of the two-device scene: a Home on this Mac (its agents are the
 * Home row's children), a connected `mini` with its own project, Home and
 * agents, and a registered device that is not connected, named at length to
 * try the rail's and the header's truncation.
 */
function deviceWorld(content: SceneContent, workspaces: Workspace[], now: number, folds: SceneFolds, busy: boolean) {
  const offlineLabel = content === "long" ? "연구실 빌드 서버 자동화 장비 (긴 이름 확인용)" : "build-box";
  const home = workspace(
    "home",
    { label: "hide", path: `${ROOT}/hide`, is_home: true, is_git: false, checkouts: [checkout({ id: "home:folder", workspace: "home", branch: null, folder: true, panes: ["h1", "h2"], marks: { turn: 1, quiet: 1 } }, now)] },
    folds,
  );
  const remoteWorkspace = (id: string, label: string, extra: Partial<Workspace>, panes: string[]): Workspace => ({
    ...workspace(id, { checkouts: [], ...extra }, folds),
    label,
    path: `/srv/${label}`,
    device_id: "mini",
    checkouts: [{ ...checkout({ id: `${id}:main`, workspace: id, branch: "main", primary: true, age: 300, panes }, now), workspace_id: id }],
  });
  const miniWorkspaces = [
    // Busy, the extra agents run in web too, so its Projects list raises them past the Needs You cap.
    remoteWorkspace("remote:mini:workspace:web", "web", {}, ["remote:mini:pane:1", "remote:mini:pane:2", ...(busy ? BUSY_AGENTS.map((row) => row.pane_id) : [])]),
    { ...remoteWorkspace("remote:mini:workspace:home", "hide", { is_home: true, is_git: false }, []), path: "/Users/example/hide" },
  ];
  return {
    home,
    devices: [
      { agent_scope: EMPTY_GALLERY_SCOPE, id: "mini", label: "mini", kind: "remote", state: "ready", message: null, ssh_alias: "mini", agent_count: 2, test: null },
      { agent_scope: EMPTY_GALLERY_SCOPE, id: OFFLINE_DEVICE, label: offlineLabel, kind: "remote", state: "unavailable", message: null, ssh_alias: OFFLINE_DEVICE, agent_count: 0, test: null },
    ],
    registrations: [
      ...workspaces.map((row) => registrationOf(row, "local")),
      registrationOf(home, "local", { home: true }),
      registrationOf(miniWorkspaces[0]!, "mini"),
      registrationOf(miniWorkspaces[1]!, "mini", { home: true }),
    ],
    remote: [
      {
        target_id: "mini",
        state: "connected",
        message: null,
        herdr_version: "0.9.1",
        session: {
          workspaces: miniWorkspaces,
          agents: busy ? [...REMOTE_AGENTS[content], ...BUSY_AGENTS] : REMOTE_AGENTS[content],
          active_tab_ids: {},
          focused_workspace_id: "remote:mini:workspace:web",
          focused_checkout_id: "remote:mini:workspace:web:main",
          focused_tab_id: "remote:mini:workspace:web:main:t1",
          focused_pane_id: null,
          pane_layouts: [],
        },
      },
      { target_id: OFFLINE_DEVICE, state: "not_connected", message: null, herdr_version: null, session: null },
    ],
  };
}

/** A purpose that names its issue the way an agent's title does, `#<number> <title>`. */
function issueTitle(issue: number, title: string): string {
  return `#${issue} ${title}`;
}

function toggled(list: string[], id: string): string[] {
  return list.includes(id) ? list.filter((item) => item !== id) : [...list, id].sort();
}

/**
 * The events the sidebar's folds dispatch, applied to the scene's folds the
 * way the core applies them. Anything else is not the scene's to model and
 * returns null, so the caller can say so instead of pretending it happened.
 */
export function applyEvent(folds: SceneFolds, event: { kind: string; payload: Record<string, unknown> }): SceneFolds | null {
  const { kind, payload } = event;
  if (kind === "checkout_agents_toggle") return { ...folds, collapsedCheckouts: toggled(folds.collapsedCheckouts, String(payload.checkout_id)) };
  if (kind === "session_fold_toggle" && payload.key === "no_agents/local") return { ...folds, noAgentsOpen: !folds.noAgentsOpen };
  if (kind === "project_checkouts_fold" && String(payload.workspace_id) in SLEEPING_PROJECTS) {
    const id = String(payload.workspace_id);
    const expanded = typeof payload.expanded === "boolean" ? payload.expanded : !folds.asleepOpen.includes(id);
    return { ...folds, asleepOpen: expanded ? [...new Set([...folds.asleepOpen, id])].sort() : folds.asleepOpen.filter((item) => item !== id) };
  }
  if (kind === "project_checkouts_fold") {
    const id = String(payload.workspace_id);
    const expanded = typeof payload.expanded === "boolean" ? payload.expanded : folds.collapsedWorkspaces.includes(id);
    return { ...folds, collapsedWorkspaces: expanded ? folds.collapsedWorkspaces.filter((item) => item !== id) : [...new Set([...folds.collapsedWorkspaces, id])].sort() };
  }
  if (kind === "agent_tree_toggle") return { ...folds, expandedAgents: toggled(folds.expandedAgents, String(payload.pane_id)) };
  if (kind === "inactive_checkouts_toggle") return { ...folds, inactiveCheckoutsOpen: toggled(folds.inactiveCheckoutsOpen, String(payload.project_path)) };
  if (kind === "inactive_projects_toggle") return { ...folds, inactiveProjectsOpen: !folds.inactiveProjectsOpen };
  if (kind === "focus_device") return { ...folds, frontDevice: String(payload.device_id) };
  return null;
}
