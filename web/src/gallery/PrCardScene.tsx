// The real PR card over invented values. No GitHub writes leave this scene.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { PrCardView } from "../PrCard";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { requireInterfaceLanguage } from "../i18n/locale";
import type { LinkedSession, PullRequest } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";

const AT = 1_792_305_660_000;
export function PrCardScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const state = params.get("state") ?? "rest";
  const fixture = useMemo(() => {
    const base = sidebarScene("reference", REFERENCE_FOLDS, AT);
    const project = base.rest.navigator!.workspaces!.find((row) => row.id === "hide")!;
    const checkout = project.checkouts[0]!;
    const failed = state === "failed", ready = state === "ready", skipped = state === "skipped";
    const pr: PullRequest = {
      number: 1372, title: "fix: let Windows hooks reach their daemon and name the step that failed", url: "https://github.com/example/hide/pull/1372", badge: "open", state: failed ? "failed" : ready || skipped ? "mergeable" : "pending", is_draft: false, review: "approved", checks: failed ? "failed" : ready || skipped ? "passing" : "pending", head_branch: "fix/windows-hook-request", base_branch: "main", closing_issues: [], created_at_unix_ms: AT, updated_at_unix_ms: AT,
      work: { author: "maintainer", author_is_bot: false, mine: true, merge_state: ready || skipped ? "clean" : "blocked", review_requests: [], asked_of_me: "none", failed_checks: failed ? [{ name: "windows check", url: "https://example.invalid/check/1" }, { name: "os contract (Windows)", url: "https://example.invalid/check/2" }] : [], pending_checks: failed || ready || skipped ? 0 : 4, check_counts: state === "unknown" ? null : { total: 18, passed: failed || skipped ? 16 : ready ? 18 : 14, failed: failed ? 2 : 0, running: failed || ready || skipped ? 0 : 3, waiting: failed || ready || skipped ? 0 : 1, skipped: skipped ? 2 : 0, unknown: 0 }, queue: { state: "not_queued" }, review_requested_at_unix_ms: null },
    };
    if (state === "unknown") { pr.checks = "unknown"; pr.state = "pending"; pr.work!.pending_checks = 0; }
    if (params.get("content") === "long") { pr.title += " - 긴한글제목과영문경로의줄바꿈을확인합니다".repeat(3); pr.head_branch += "/long-branch-name".repeat(8); }
    checkout.branch = pr.head_branch!;
    checkout.pull_request = pr;
    checkout.github = { available: true, loading: false, stale: false, failure_category: null, last_success_at_unix_ms: AT };
    project.pull_requests = [pr];
    project.agent_scope!.prs = { rows: [{ number: pr.number, group: failed ? "blocked" : ready || skipped ? "needs_you" : "waiting", fold: null, reason: { kind: failed ? "failed" : ready || skipped ? "merge" : "ci", count: failed ? 2 : 4, base: "main", since_unix_ms: AT }, agents: [], agent_pane: null, checkout_id: checkout.id, checkout_ids: [checkout.id], issue: null }], groups: [{ group: failed ? "blocked" : ready || skipped ? "needs_you" : "waiting", numbers: [pr.number] }], needs_you: ready || skipped ? 1 : 0 };

    base.rest.pr_work = { link: null, feedback: null, cards: [{ workspace_id: project.id, number: pr.number, reading: state === "loading", head: "a".repeat(40), body: "## Summary\nWindows에서 에이전트 훅이 데몬에 요청을 전달하지 못해 메모리와 편지가 프롬프트에 도달하지 않았습니다.\n\nWindows 파이프의 쓰기 제한 시간을 지키고 실패한 요청 단계를 구분하도록 변경했습니다.\n\n## Review\nWindows 파이프 연결과 실패 단계 기록을 검토합니다.\n\n## Evidence\n레이아웃 확인을 위한 예시입니다.", files: [{ path: "hide-platform/src/ipc/windows.rs", additions: 400, deletions: 100, generated: false }, { path: "hide-agent-hooks/src/client.rs", additions: 340, deletions: 101, generated: false }], files_total: 2, threads: [], merge: { can_merge: true, methods: ["merge"], queue: false }, failed: [], write: null }] };
    // The timeline's sessions: a waiting one, an asking one and an ended one, joined to the scene's panes by session id.
    const live = (pane: string, session: string) => { base.rest.navigator!.agents!.find((agent) => agent.pane_id === pane)!.session_id = session; };
    live("a1c3", "s-waiting");
    live("a3", "s-asking");
    const session = (id: string, agent: string, started: number, ended: number | null, role: "created" | "worked"): LinkedSession => ({ agent, id, ids: [id], device_id: "local", role, pr: pr.number, request: "Windows 훅 요청 경로 고치기", started_at_unix_ms: started, ended_at_unix_ms: ended, path: `/sessions/${id}`, cwd: checkout.path, file: "present", parent: null });
    const sessions = [session("s-ended", "codex", AT - 7_200_000, AT - 3_600_000, "created"), session("s-waiting", "claude", AT - 3_000_000, null, "worked"), session("s-asking", "claude", AT - 1_800_000, null, "worked")];
    return { base, project, pr, sessions };
  }, [params, state]);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest: fixture.base.rest, agents: [], connection: "live", linkPanel: { workspace_id: fixture.project.id, target: { kind: "pr", number: 1372 }, loading: false, failure: null, pr: { number: 1372, branch: fixture.pr.head_branch!, title: fixture.pr.title, url: fixture.pr.url, created_at_unix_ms: AT, closed_at_unix_ms: null, merged_at_unix_ms: null, state: fixture.pr.state, issues: [], worktrees: [] }, prs: [], sessions: fixture.sessions, total: fixture.sessions.length } });
    useUiStore.setState({ screen: { kind: "workspace" } });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", params.get("scale") ?? "1");
  }, [fixture, theme, params]);
  return <TooltipProvider><div style={{ width: Number(params.get("width") ?? 680) }} className="flex h-full min-w-0 flex-col bg-background text-foreground" data-gallery-scene="pr-card"><PrCardView workspaceId={fixture.project.id} number={1372} actions={actions} /></div></TooltipProvider>;
}
