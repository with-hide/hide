// Invented missions for the right lane's Mission tool states (PRD mission-tool
// B3-B11): one waiting on a decision, one at work, one on a device that does
// not answer, one waiting on its merge order, and finished and cancelled ones
// past the three the list shows. Every name, number and time is made up.
import type { ConductorSettings, Mission, MissionNode } from "../snapshot";

const MINUTE = 60_000;
const DAY = 24 * 60 * MINUTE;

function node(key: string, title: string, extra: Partial<MissionNode> = {}): MissionNode {
  return { key, title, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, verdict: null, checks: null, ...extra };
}

function mission(id: string, title: string, workspace: string, path: string, extra: Partial<Mission>): Mission {
  return { id, title, workspace_id: workspace, project: path, checkout_path: path, device_id: "local", status: "running", scope: "pr", cap: 3, exempt: false, strength: "normal", other_vendor: true, conductor: { pane_id: "w1:p1", name: "conductor", agent_kind: "claude", session: null, present: true, takes_answers: true }, nodes: [], merge_order: [], check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 0, started_at_unix_ms: null, closed_at_unix_ms: null, running: 0, stale: false, github_stale: false, github_read_at_unix_ms: null, ...extra };
}

const claude = { agent: "claude", model: "Opus", effort: "high" };
const codex = { agent: "codex", model: "Astra", effort: "xhigh" };

/** The project's missions for the list state; `now` anchors their times. */
export function missionToolMissions(workspace: string, path: string, now: number): Mission[] {
  const started = (minutes: number) => ({ created_at_unix_ms: now - minutes * MINUTE, started_at_unix_ms: now - minutes * MINUTE });
  const closed = (days: number) => ({ created_at_unix_ms: now - days * DAY - 90 * MINUTE, started_at_unix_ms: now - days * DAY - 90 * MINUTE, closed_at_unix_ms: now - days * DAY });
  return [
    mission("m-tabs", "설정 화면 탭 구조", workspace, path, {
      ...started(42), decision: "설정 탭 순서를 지금 메뉴 순서로 둘까요, 자주 쓰는 순서로 바꿀까요?", running: 1,
      nodes: [
        node("shell", "탭 셸 컴포넌트", { status: "done", worker: claude, pr: 1901, pr_state: "merged" }),
        node("store", "설정 저장 경로", { status: "done", worker: codex, pr: 1902, pr_state: "merged" }),
        node("move", "탭별 화면 옮기기", { status: "working", worker: claude, depends_on: ["shell", "store"] }),
        node("order", "탭 순서", { status: "held", depends_on: ["move"] }),
        node("demo", "시연 검증", { kind: "verify", depends_on: ["move"] }),
      ],
    }),
    mission("m-ci", "릴리스 CI 시간 줄이기", workspace, path, {
      ...started(64), running: 2,
      nodes: [
        node("cache", "의존성 캐시", { status: "done", worker: codex, pr: 1905, pr_state: "merged" }),
        node("mini", "mac 잡 mini로 옮기기", { status: "working", worker: claude }),
        node("split", "테스트 샤드 나누기", { status: "review", worker: codex }),
        node("check", "시간 재기", { kind: "verify", depends_on: ["mini", "split"] }),
      ],
    }),
    mission("m-remote", "원격 기기 로그 모으기", workspace, path, {
      ...started(130), stale: true, running: 1,
      nodes: [node("collect", "로그 수집기", { status: "working", worker: claude }), node("view", "로그 보기", { depends_on: ["collect"] })],
    }),
    mission("m-stage", "Atorni Stage 1.2", workspace, path, {
      ...started(180), status: "merge_wait", merge_order: ["api", "ui"],
      nodes: [
        node("api", "자료 확정 API", { status: "done", worker: codex, pr: 1911, pr_state: "mergeable" }),
        node("ui", "확정 화면", { status: "done", worker: claude, pr: 1912, pr_state: "pending", base_pr: 1911, depends_on: ["api"] }),
      ],
    }),
    mission("m-case", "Atorni Stage 1.1 자동 확정과 건 화면", workspace, path, { ...closed(1), status: "done", nodes: [node("a", "건 보기 화면", { status: "done", pr: 1871, pr_state: "merged" })] }),
    mission("m-release", "릴리스 v1.0.4 준비", workspace, path, { ...closed(2), status: "done", nodes: [node("a", "버전 올리기", { status: "done" })] }),
    mission("m-mail", "원격 메일함 정리", workspace, path, { ...closed(3), status: "cancelled", nodes: [node("a", "메일함 정리", { status: "working" })] }),
    mission("m-icons", "레일 아이콘 다듬기", workspace, path, { ...closed(6), status: "done", nodes: [node("a", "아이콘", { status: "done" })] }),
  ];
}

/** Settings › Conductor's values as the defaults show them. */
export const MISSION_TOOL_DEFAULTS: ConductorSettings = {
  concurrency: 3,
  default_scope: "pr",
  default_strength: "normal",
  other_vendor: true,
  candidates: [
    { agent: "claude", model: "opus", effort: "high", purpose: "Conductor" },
    { agent: "codex", model: null, effort: null, purpose: "검증" },
  ],
};
