// Production components over fixed, invented wire values. These examples
// compare typography and geometry; runtime policy is tested in herdr-core.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TaskLens } from "../TaskLens";
import { TooltipProvider } from "../components/ui/tooltip";
import type { TaskState } from "../agentScope";
import { PaneView } from "../PaneView";
import type { AgentRow, AgentState, PaneHeader, PrState, RaisedAsk, SessionGroup } from "../snapshot";
import type { RowMark, StatusTone, RequestVerb } from "../snapshot";
import { useShellStore } from "../store";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";
import type { SceneParams } from "./SidebarScene";

type Example = { title: string; group: SessionGroup; line: string | null; ask?: NonNullable<AgentState["ask"]>; pr?: PrState; unfinished?: boolean; mark?: AgentRow["descendant_mark"] };
/** Each example's task: its row's index and the state the core gave it. */
const TASKS: [number, TaskState][] = [[0, "needs_you"], [1, "needs_you"], [2, "needs_you"], [4, "working"], [5, "waiting"], [3, "done"], [6, "resting"], [7, "resting"], [8, "resting"]];
const EXAMPLES: Example[] = [
  { title: "입력과 세션 복귀 흐름 검토", group: "needs_you", line: null, ask: { verb: "approval", what: "검증 명령 실행 권한이 필요합니다", more: 0 } },
  { title: "한국어 입력 경계 검토", group: "needs_you", line: null, ask: { verb: "answer", what: "기존 stdin 종료 경로도 남길까요?", more: 0 } },
  { title: "자식 검토에서 답이 필요함", group: "needs_you", line: null, ask: { verb: "confirm", what: "입력 경계 검토", more: 1 } },
  { title: "세션 상태 투영 정리", group: "done", line: "투영 정리를 끝냈습니다", pr: "mergeable" },
  { title: "오래된 PR의 검토", group: "working", line: "실패한 검증을 고치는 중", pr: "failed" },
  { title: "세션 패널 구현", group: "working", line: "프로젝트 범위를 연결하는 중", mark: { kind: "working", count: 3 } },
  { title: "회귀 테스트", group: "idle", line: "남은 경계 테스트 2개", unfinished: true, pr: "pending" },
  { title: "쉬는 세션", group: "idle", line: null },
  { title: "완료한 검토", group: "resolved", line: null, pr: "merged" },
];
const RAISED: RaisedAsk = { verb: "approval", what: "검증 명령 실행 권한이 필요합니다", raised_pane_id: "a1c1", pane_id: "a1c1", title: "한국어 입력 경계 검토", agent_kind: "codex", open_pane_id: "a1c1", since_unix_ms: null, path: ["입력과 세션 복귀 흐름 검토", "한국어 입력 경계 검토"], checkout: "prd/session-ui", human_notice: true };
const BANDS: [string, PaneHeader["band"]][] = [
  ["sleeping", { kind: "sleeping", tone: "muted", reason: "12분 동안 휴면 중", since_unix_ms: null, action: null, more: 0, exit_code: null }],
  ["failed", { kind: "failed", tone: "error", reason: "세션을 찾을 수 없음", since_unix_ms: null, action: null, more: 0, exit_code: null }],
  ["exit", { kind: "exit", tone: "error", reason: null, since_unix_ms: null, action: null, more: 0, exit_code: 1 }],
  ["device", { kind: "device_offline", tone: "muted", reason: "mini", since_unix_ms: null, action: null, more: 0, exit_code: null }],
  ["approval", { kind: "approval", tone: "warning", reason: null, since_unix_ms: null, action: null, more: 0, exit_code: null }],
  ["answer", { kind: "answer", tone: "warning", reason: "기존 stdin 종료 경로도 남길까요?", since_unix_ms: null, action: null, more: 0, exit_code: null }],
  ["fix", { kind: "fix", tone: "error", reason: null, since_unix_ms: null, action: { kind: "pr", workspace_id: "hide", url: "https://github.com/acme/app/pull/221", number: 221, checks: "failed", tone: "error" }, more: 0, exit_code: null, facts: { kind: "pull_request", checks: "failed", review: null } }],
  ["merge", { kind: "merge", tone: "pr", reason: null, since_unix_ms: null, action: { kind: "pr", workspace_id: "hide", url: "https://github.com/acme/app/pull/221", number: 221, checks: "passing", tone: "pr" }, more: 0, exit_code: null, facts: { kind: "pull_request", checks: "passing", review: "approved" } }],
  ["raised", { kind: "raised", tone: "warning", reason: null, since_unix_ms: null, action: { kind: "child", pane_id: "a1c1", label: RAISED.title }, more: 1, exit_code: null, raised: RAISED }],
  ["result", { kind: "result", tone: "success", reason: "보고서가 준비됐습니다", since_unix_ms: null, action: null, more: 0, exit_code: null }],
  ["working", null], ["idle", null], ["children", null], ["unknown", null],
];

// Panes drawn as a delegated child, so their header starts with the path back to the root (B20).
const CHILD_PANES = ["sleeping", "fix", "merge", "result", "working"];

// Frozen visual examples, not a second implementation of core judgments.
const MARKS: [RowMark, StatusTone["kind"], RequestVerb][] = [
  ["turn", "warning", "answer"], ["turn", "warning", "answer"], ["turn", "warning", "answer"],
  ["done", "success", "review"], ["done", "success", "review"], ["going", "working", "working"],
  ["going", "working", "waiting"], ["quiet", "subtle", "idle"], ["quiet", "subtle", "result"],
];

export function SessionWorkflowScene({ scene: kind, theme, content, scale, width }: SceneParams & { scene: "session-panel" | "pane-header" }) {
  const actions = useMemo(() => createActions(() => true), []);
  const fixture = useMemo(() => {
    const base = sidebarScene(content, REFERENCE_FOLDS, Date.now());
    const project = base.rest.navigator!.workspaces!.find((row) => row.id === "hide")!;
    const checkout = project.checkouts[0]!;
    const pane = checkout.tabs[0]!.panes[0]!;
    const seed = base.agents.find((row) => row.pane_id === "a1")!;
    const agents: AgentRow[] = EXAMPLES.map((example, index) => {
      const [mark, tone, verb] = MARKS[index]!;
      return {
        ...seed, id: `s${index}`, pane_id: `s${index}`, mark, agent_kind: index % 2 ? "codex" : "claude",
        identity_label: content === "long" ? `${example.title} - 긴 한국어 제목과 변경 사항을 좁은 화면에서 확인하는 작업` : example.title,
        lineage_child_pane_ids: index === 5 ? ["a1c1", "a1c2"] : [],
        // One Working root is a fork of the row beside it (PRD fork-operator-root B8).
        forked_from: index === 4 ? { pane_id: "s5", label: EXAMPLES[5]!.title } : null,
        descendant_mark: example.mark ?? null,
        state: { ...seed.state, verb, mark_tone: { kind: tone, read: false }, chip_tone: { kind: tone, read: false }, session: { group: example.group, line: example.line, unfinished: example.unfinished ?? false }, ask: example.ask ?? null, tree_rank: 1,
          pr: example.pr ? { count: 1, worst: example.pr, pulls: [{ index: 0, state: example.pr }] } : null,
          request_since: Date.now() - 180_000, line: example.line ? { text: example.line, mode: "request", tone: { kind: "subtle", read: false } } : null },
        request: { verb, verb_since_unix_ms: Date.now(), line: example.line ?? undefined, request: null, later_by: null, reply: null, pull_requests: example.pr ? [{ number: 220 + index, title: example.title, url: `https://example.invalid/pull/${220 + index}`, badge: example.pr === "merged" ? "merged" : "open", checks: example.pr === "failed" ? "failed" : example.pr === "pending" ? "pending" : "passing", review: example.pr === "mergeable" ? "approved" : "review_required", head_branch: "prd/session-ui", closing_issues: [], live: true, duty: true, created: true, settled_at_unix_ms: null }] : [] },
        resolved: example.group === "resolved" ? { at_unix_ms: Date.now(), source: "operator" } : null,
      };
    });
    const feature = { ...checkout, id: "session-ui", label: "prd/session-ui", branch: "prd/session-ui" };
    const scope = { ...project.agent_scope, members: agents.map((agent, index) => ({ pane_id: agent.pane_id, project_id: project.id, checkout_id: index === 5 ? checkout.id : feature.id })), work: { s3: { pull: 0, more: 0, issues: [], issue_chips: [] }, s4: { pull: 0, more: 0, issues: [], issue_chips: [] }, s5: { pull: null, more: 0, issues: ["issue-219"], issue_chips: ["issue-219"] }, s6: { pull: 0, more: 0, issues: [], issue_chips: [] } }, tasks: { cards: TASKS.map(([member, state]) => ({ root: agents[member]!.pane_id, state, members: [agents[member]!.pane_id], checkouts: [member === 5 ? checkout.id : feature.id], recency: "" })), needs_you: 3, recency: "" } };
    project.agent_scope = scope;
    project.tasks = { source: null, overflow: false, tasks: [{ key: "issue-219", source: "github", id: `#${219}`, url: "https://example.invalid/issue/219", title: "세션 패널 구현", open: true }] };
    checkout.agent_scope = { ...scope, members: [scope.members[5]!] };
    // The sidebar's 태스크 list reads the device's tasks.
    for (const device of base.rest.navigator?.devices ?? []) device.agent_scope = scope;
    const panes = agents.map((agent) => ({ ...pane, id: agent.pane_id, identity_label: agent.identity_label }));
    checkout.tabs = [{ ...checkout.tabs[0]!, panes: [panes[5]!] }];
    feature.tabs = [{ ...checkout.tabs[0]!, id: "session-ui-tab", panes: panes.filter((_, index) => index !== 5) }];
    project.checkouts = [checkout, feature];
    base.rest.navigator!.workspaces = [project];
    base.rest.navigator!.focused_checkout_id = checkout.id;
    base.rest.navigator!.focused_workspace_id = project.id;
    // The panel's tree opens on the row with children, so a child's columns are compared with its root's.
    base.rest.ui_state = { ...base.rest.ui_state!, sessions_expanded_agent_pane_ids: ["s5"] };
    // A has a different-checkout child, a same-checkout child, and a folded
    // grandchild with a PR. The parent's count includes all three.
    const children = base.agents.filter((row) => ["a1c1", "a1c2", "a1g1"].includes(row.pane_id)).map((child, index): AgentRow => {
      const state: PrState = index === 1 ? "failed" : index === 2 ? "mergeable" : "pending";
      const number = 230 + index;
      const line = index === 0 ? "브랜치와 진행 설명을 분리하는 중" : index === 2 ? "좁은 화면 검증을 끝냈습니다" : null;
      return { ...child, lineage_parent_pane_id: index === 2 ? "a1c1" : "s5", lineage_child_pane_ids: index === 0 ? ["a1g1"] : [],
        state: { ...child.state, session: { group: index === 0 ? "working" : "idle", line, unfinished: false }, pr: { count: 1, worst: state, pulls: [{ index: 0, state }] }, branch_badge: index === 0 ? "feat/session-row-clarity-and-branch-label" : null },
        request: { verb: index === 0 ? "working" : "idle", verb_since_unix_ms: Date.now(), request: null, later_by: null, reply: null, pull_requests: [{ number, title: child.identity_label, url: `https://example.invalid/pull/${number}`, badge: "open", checks: state === "failed" ? "failed" : state === "pending" ? "pending" : "passing", review: state === "mergeable" ? "approved" : "review_required", head_branch: "feat/session-row-clarity-and-branch-label", closing_issues: [], live: true, duty: true, created: true, settled_at_unix_ms: null }] } };
    });
    const childPanes = children.map((child) => ({ ...pane, id: child.pane_id, identity_label: child.identity_label }));
    feature.tabs[0]!.panes.push(childPanes[0]!);
    checkout.tabs[0]!.panes.push(...childPanes.slice(1));
    const headers = Object.fromEntries(BANDS.map(([id, band]) => [id, { working: id === "working", band: band && { ...band, since_unix_ms: ["approval", "answer", "fix", "merge", "raised", "result"].includes(id) ? Date.now() - 180_000 : band.since_unix_ms } }]));
    // The fix and merge panes own one PR each, so their header shows the own PR chip (B21, B22).
    const ownPr = (id: string): Pick<AgentRow, "state" | "request"> => {
      const state = id === "fix" ? "failed" as const : "mergeable" as const;
      return {
        state: { ...seed.state, pr: { count: 1, worst: state, pulls: [{ index: 0, state }] } },
        request: { verb: "working", verb_since_unix_ms: Date.now(), line: undefined, request: null, later_by: null, reply: null, pull_requests: [{ number: 221, title: "세션 상태 투영 정리", url: "https://github.com/acme/app/pull/221", badge: "open", checks: state === "failed" ? "failed" : "passing", review: "approved", head_branch: "prd/session-ui", closing_issues: [], live: true, duty: true, created: true, settled_at_unix_ms: null }] },
      };
    };
    // B2's full aggregate, unknown and empty examples use explicit snapshot
    // facts, not a gallery implementation of the core's status decisions.
    const childExamples: AgentRow[] = [
      { ...children[0]!, id: "b2-working", pane_id: "b2-working", identity_label: "자식 상태 칩과 팝업 연결 구현", descendant_mark: null, state: { ...children[0]!.state, tree_rank: 1, line: { text: "작업 상태 칩과 팝업을 연결하는 중", mode: "news", tone: { kind: "working", read: false } } } },
      { ...children[1]!, id: "b2-result", pane_id: "b2-result", identity_label: "헤더 배치 검토", mark: "done", status_code: "done", unknown: false, descendant_mark: null, state: { ...children[1]!.state, tree_rank: 2, line: null } },
      { ...children[1]!, id: "b2-idle", pane_id: "b2-idle", identity_label: "다음 요청 대기", unknown: false, descendant_mark: null, state: { ...children[1]!.state, tree_rank: 3, line: null } },
      { ...children[1]!, id: "b2-unknown", pane_id: "b2-unknown", identity_label: "상태 확인 중인 자식", mark: "unknown", status_code: "unknown", activity: "unknown", demand: "none", descendant_mark: null, state: { ...children[1]!.state, tree_rank: 3, line: null } },
      { ...children[1]!, id: "b2-question", pane_id: "b2-question", identity_label: "부모에게 PR 구성 질문", mark: "turn", status_code: "question", unknown: false, descendant_mark: null, state: { ...children[1]!.state, tree_rank: 0, line: { text: "기존 PR에 합칠지 부모 응답을 기다리는 중", mode: "quiet", tone: { kind: "subtle", read: false } } } },
    ];
    const headerAgents = BANDS.map(([id]) => ({ ...seed, id, pane_id: id,
      device_id: id === "device" ? "mini" : seed.device_id,
      descendant_mark: id === "raised" ? { kind: "raised" as const, count: 2 } : null,
      raised: id === "raised" ? [RAISED, { ...RAISED, raised_pane_id: "a1c2", pane_id: "a1c2", open_pane_id: "a1c2", title: children[1]!.identity_label, verb: "answer" as const, what: "PR을 함께 올릴까요?" }] : [],
      lineage_child_pane_ids: id === "idle" ? [] : id === "children" ? childExamples.map((child) => child.pane_id) : id === "unknown" ? ["b2-unknown"] : ["a1c1", "a1c2"],
      direct_child_counts: id === "children" ? { error: 0, approval: 0, question: 1, working: 1, done: 1 } : id === "unknown" ? { error: 0, approval: 0, question: 0, working: 0, done: 0 } : { error: 0, approval: 0, question: 0, working: 1, done: 0 },
      ...(id === "fix" || id === "merge" ? ownPr(id) : {}) }));
    base.rest.terminal = { headers };
    base.rest.status = { ...base.rest.status, remote: [{ target_id: "mini", state: "not_connected", message: "Mac mini 연결되지 않음", herdr_version: null, session: null }] };
    return { ...base, agents: [...agents, ...children, ...childExamples, ...headerAgents], pane };
  }, [content]);
  useLayoutEffect(() => {
    useShellStore.setState({ rest: fixture.rest, agents: fixture.agents, connection: "gone" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
  }, [fixture, theme, scale]);
  return <TooltipProvider>{kind === "session-panel" ? <div className="flex h-full bg-background text-foreground" data-gallery-scene={kind}><TaskLens actions={actions} /></div> : <div style={{ width: width ?? undefined }} className="grid grid-cols-2 gap-lg bg-background p-xl text-foreground" data-gallery-scene={kind}>{BANDS.map(([id]) => <div key={id} className="h-[var(--size-pane-state-example)] min-w-0 border border-border"><PaneView pane={{ ...fixture.pane, id, identity_label: content === "long" ? "긴 한국어 제목을 좁은 pane에서 확인하는 상태 검토" : "한국어 입력 경계 검토", fork: id === "idle" ? { available: true, forked_from: { pane_id: "a1", label: "hide 에이전트 지원 PR 묶음 머지 조율" } } : undefined, lineage_path: CHILD_PANES.includes(id) ? [{ pane_id: "a1", label: "hide 에이전트 지원 PR 묶음 머지 조율", siblings: [] }, { pane_id: id, label: "한국어 입력 경계 검토", siblings: [] }] : undefined }} transport={undefined} focused={false} scale={1} actions={actions} agentKind="codex" markSymbol="quiet" markTone="text-muted-foreground" paneCount={2} zoomed={false} /></div>)}</div>}</TooltipProvider>;
}
