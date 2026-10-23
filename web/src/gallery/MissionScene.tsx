// The real mission map and Settings › Conductor over invented values (PRD
// software-factory-conductor), one state per `state` parameter: plan,
// running, decision, stack, no-conductor, stale, failed, settings. The
// Conductor pane's head with its mission chip and pinned small map (PRD
// mission-pane-chip) is `pane-running`, `pane-decision` and `pane-first-plan`.
// Nothing leaves this scene.
import { useLayoutEffect, useMemo } from "react";
import { AgentMark } from "../AgentMark";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { requireInterfaceLanguage } from "../i18n/locale";
import { MissionMap } from "../MissionMap";
import { PaneWorkChip } from "../PaneMissionChip";
import { ConductorTab } from "../settings/ConductorTab";
import type { AgentRow, Mission, MissionNode, SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";

/** The Conductor's pane, as its agent row and the mission name it. */
const CONDUCTOR_PANE = "w1:p1";
/** A pane state reads the map state it draws small. */
const PANE_STATES: Record<string, string> = { "pane-running": "running", "pane-decision": "decision", "pane-first-plan": "plan" };

const MINUTE = 60_000;
/** An issue or pull request number as the boards write it, built so no literal reads as a color. */
const no = (n: number) => `#${n}`;

function node(key: string, title: string, extra: Partial<MissionNode>): MissionNode {
  return { key, title, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: null, started_at_unix_ms: null, finished_at_unix_ms: null, ...extra };
}

const opus = (effort: string | null) => ({ agent: "claude", model: "Opus", effort, effort_auto: true });
const astra = (effort: string | null) => ({ agent: "codex", model: "Astra", effort, effort_auto: true });
const sonnet = { agent: "claude", model: "Sonnet", effort: "low", effort_auto: false };
/** The screen node's pane, whose recorded launch runs another model than the plan named (the card shows the pane's). */
const SCREEN_PANE = "w1:p7";

/** The eight nodes of the boards (cmm1-cmm7): four starts, two that wait, one check together. */
function plan(state: string, now: number): MissionNode[] {
  const ago = (minutes: number) => now - minutes * MINUTE;
  const planned = state === "plan";
  const done = (minutes: number, length: number) => ({ status: "done" as const, started_at_unix_ms: ago(minutes), finished_at_unix_ms: ago(minutes - length) });
  const working = (minutes: number) => ({ status: "working" as const, started_at_unix_ms: ago(minutes) });
  if (planned) {
    return [
      node("cause", `${no(1487)} 원인 조사`, { kind: "research", delivery: null, worker: opus(null) }),
      node("fix-1481", `이슈 ${no(1481)} 수정`, { worker: astra(null) }),
      node("fix-1483", `이슈 ${no(1483)} 수정`, { worker: astra(null) }),
      node("api", "API 추가", { worker: astra(null) }),
      node("fix-1487", `${no(1487)} 수정`, { depends_on: ["cause"], worker: astra(null) }),
      node("screen", "API 쓰는 화면", { depends_on: ["api"], worker: opus(null) }),
      node("together", "함께 확인", { kind: "verify", delivery: null, depends_on: ["screen", "api"], worker: sonnet }),
    ];
  }
  const stack = state === "stack" || state === "no-conductor";
  return [
    node("cause", `${no(1487)} 원인 조사`, { kind: "research", delivery: null, worker: opus("xhigh"), ...done(44, 24), line: "조사 · 결론 반영" }),
    node("fix-1481", `이슈 ${no(1481)} 수정`, { worker: astra("high"), ...done(40, 14), pr: 1501, pr_state: "mergeable", stage: 3 }),
    node("fix-1490", `이슈 ${no(1490)} 수정`, { worker: astra("medium"), ...done(40, 9), pr: 1505, pr_state: "mergeable", stage: 3, change: `${no(1483)}은 ${no(1490)}과 같은 원인이라 합침` }),
    node("api", "API 추가", { worker: astra("high"), ...done(40, 17), pr: 1503, pr_state: state === "no-conductor" ? "merged" : "mergeable", stage: state === "no-conductor" ? 4 : 3 }),
    node("fix-1487", `${no(1487)} 수정`, { depends_on: ["cause"], worker: astra("medium"), ...(stack ? done(30, 12) : working(5)), pr: 1506, pr_state: stack ? "mergeable" : "pending", stage: stack ? 3 : 3 }),
    state === "decision"
      ? node("screen", "API 쓰는 화면", { depends_on: ["api"], worker: astra("high"), status: "failed", started_at_unix_ms: ago(21), pr: 1504, base_pr: 1503, pr_state: "failed", stage: 3, attempts: 2, line: "CI 실패 · 시도 2/2", review: "리뷰 전 · CI 통과 뒤 시작" })
      : node("screen", "API 쓰는 화면", { depends_on: ["api"], worker: astra("high"), pane_id: stack ? null : SCREEN_PANE, ...(stack ? done(33, 33) : working(12)), status: stack ? "waiting" : "working", pr: 1504, base_pr: 1503, pr_state: state === "no-conductor" ? "pending" : "mergeable", stage: state === "no-conductor" ? 2 : 3, line: state === "no-conductor" ? "옮기기 필요" : stack ? `아래 ${no(1503)} 머지 대기` : null }),
    node("together", "함께 확인", { kind: "verify", delivery: null, depends_on: ["screen", "api"], worker: sonnet, ...(stack ? done(8, 4) : {}), line: stack ? "합친 변경 ✓" : null }),
  ];
}

function scene(state: string, now: number): { mission: Mission; rest: SnapshotRest } {
  const nodes = plan(state, now);
  const mission: Mission = {
    id: "mission-1", title: "이슈 정리", workspace_id: "hide", project: "/projects/herdr-ide", checkout_path: "/projects/herdr-ide", device_id: "local",
    status: state === "plan" ? "planning" : state === "stack" ? "merge_wait" : "running",
    scope: "pr", cap: 4,
    conductor: { pane_id: CONDUCTOR_PANE, name: "이슈 정리 · Conductor", agent_kind: "claude", session: { device_id: "local", agent: "claude", session_id: "scene-conductor" }, present: state !== "no-conductor" },
    nodes, merge_order: ["api", "screen"], check: "cargo test && pnpm test", github: "octocat",
    decision: state === "decision" ? "화면 PR 방식 선택" : state === "no-conductor" ? `${no(1503)}이 머지되어 ${no(1504)}를 옮겨야 해요` : null,
    delay: null, report: null,
    created_at_unix_ms: now - 50 * MINUTE, started_at_unix_ms: state === "plan" ? null : now - (state === "stack" ? 72 : 44) * MINUTE, closed_at_unix_ms: state === "stack" ? now : null,
    running: state === "decision" ? 1 : state === "running" ? 4 : 0,
    // A device that disconnected leaves its last values, dimmed (B20).
    stale: state === "stale", github_stale: state === "stale", github_read_at_unix_ms: state === "stale" ? now - 25 * MINUTE : now,
  };
  const rest = {
    missions: state === "failed" ? { items: [], failed: "unreadable" } : { items: [mission] },
    navigator: { devices: [{ id: "local", kind: "local", label: "This Mac", kit: { agents: [
      { id: "claude-code", label: "Claude Code", availability: "available", enabled: true },
      { id: "codex", label: "Codex", availability: "available", enabled: true },
      { id: "pi", label: "Pi", availability: "available", enabled: true },
      { id: "grok", label: "Grok", availability: "not_installed", enabled: false },
    ] } }], workspaces: [] },
    overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: [{ device_id: "local", pane_id: SCREEN_PANE, session_digest: "scene-screen", launch_model: "Sol", launch_effort: null }] },
    status: { remote: [], background_ai: { enabled: true, providers: [
      { agent: "claude-code", state: "ready", models: ["Opus 5.5", "Sonnet 5.5", "Haiku 5.5"] },
      { agent: "codex", state: "ready", models: ["GPT-6-Astra", "GPT-5.6-Sol"] },
    ] } },
    ui_state: { conductor_settings: { concurrency: 4, default_scope: "pr", candidates: [
      { agent: "claude", model: "Opus 5.5", effort: null, purpose: "화면·설계가 섞인 작업" },
      { agent: "codex", model: "GPT-6-Astra", effort: null, purpose: "구현, 테스트 고치기" },
      { agent: "claude", model: "Sonnet 5.5", effort: "low", purpose: "리뷰, 문서" },
      { agent: "pi", effort: null, purpose: "짧은 조사" },
      { agent: "grok", effort: null, purpose: "" },
    ] } },
  } as unknown as SnapshotRest;
  return { mission, rest };
}

export function MissionScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const state = params.get("state") ?? "running";
  const pane = PANE_STATES[state];
  const fixture = useMemo(() => scene(pane ?? state, Date.now()), [pane, state]);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    const agents = pane ? [{ pane_id: CONDUCTOR_PANE, device_id: "local" } as unknown as AgentRow] : [];
    useShellStore.setState({ rest: fixture.rest, agents, connection: "live" });
    // The small map pinned, as a press, the Show plan chord or a first plan watched leaves it.
    useUiStore.setState({ planPane: pane ? CONDUCTOR_PANE : null });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", params.get("scale") ?? "1");
  }, [fixture, pane, theme, params]);
  const width = Number(params.get("width") ?? (state === "settings" ? 900 : 1080));
  return (
    <TooltipProvider>
      <div style={{ width }} className="flex h-full min-w-0 flex-col bg-background text-foreground" data-gallery-scene="mission" data-mission-scene={state}>
        {state === "settings" ? (
          <div className="p-xl"><ConductorTab actions={actions} /></div>
        ) : pane ? (
          <div className="flex h-(--size-pane-header) min-w-0 items-center gap-xs border-b border-border bg-muted px-sm text-caption" data-mission-scene-pane="true">
            <AgentMark kind="claude" />
            <span className="truncate">Conductor</span>
            <PaneWorkChip paneId={CONDUCTOR_PANE} actions={actions} />
          </div>
        ) : (
          <MissionMap missionId="mission-1" actions={actions} />
        )}
      </div>
    </TooltipProvider>
  );
}
