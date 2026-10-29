// Production components over fixed, invented wire values. These examples
// compare typography and geometry; runtime policy is tested in herdr-core.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TaskLens } from "../TaskLens";
import { TooltipProvider } from "../components/ui/tooltip";
import type { TaskState } from "../agentScope";
import { PaneView } from "../PaneView";
import type { AgentRow, AgentState, AgentWait, Mission, PaneConnection, PaneHeader, PrState, RaisedAsk, SessionGroup } from "../snapshot";
import type { RowMark, StatusTone, RequestVerb } from "../snapshot";
import { useShellStore } from "../store";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";
import type { SceneParams } from "./SidebarScene";

type Example = { title: string; group: SessionGroup; line: string | null; ask?: NonNullable<AgentState["ask"]>; pr?: PrState; unfinished?: boolean; mark?: AgentRow["descendant_mark"]; read?: boolean; wait?: AgentWait; pull?: number };
/** Each example's task: its row's index and the state the core gave it. */
const TASKS: [number, TaskState][] = [[0, "needs_you"], [1, "needs_you"], [2, "needs_you"], [4, "working"], [5, "waiting"], [3, "done"], [6, "revisit"], [7, "resting"], [8, "resting"], [9, "needs_you"], [10, "needs_you"], [11, "waiting"], [12, "waiting"]];
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
  // needs-you-cleanup: a Conductor resting on its mission's decision, a question the operator read, and two waits that name their cause.
  { title: "사이드바 정리 미션", group: "needs_you", line: null, ask: { verb: "answer", what: null, more: 0, subject: { cause: "mission_decision", line: "뼈대의 check 줄을 건너뛸까요?" } } },
  { title: "배포 환경 선택", group: "needs_you", line: null, ask: { verb: "answer", what: "staging부터 올릴까요?", more: 0 }, read: true },
  { title: "검토 나눠 맡기기", group: "working", line: null, wait: "children" },
  { title: "PR 체크 기다리기", group: "working", line: null, wait: "checks", pull: 1846 },
];
const RAISED: RaisedAsk = { verb: "approval", what: "검증 명령 실행 권한이 필요합니다", raised_pane_id: "a1c1", origin_title: "한국어 입력 경계 검토", pane_id: "a1c1", title: "한국어 입력 경계 검토", agent_kind: "codex", open_pane_id: "a1c1", since_unix_ms: null, path: ["입력과 세션 복귀 흐름 검토", "한국어 입력 경계 검토"], checkout: "prd/session-ui", human_notice: true };
const PANE_RAISED: RaisedAsk = { ...RAISED, raised_pane_id: "pane-raised-child", origin_title: "UI 검증", pane_id: "pane-raised-child", open_pane_id: "pane-raised-child", title: "UI 검증", what: "권한 승인을 기다리고 있어요", path: ["Group tab 기능 설계", "UI 검증"] };
const NO_NOTICE = { notice: null, band: null };
const TURN_REQUEST = { text: "제안대로 진행 + 애매한 상태 더 점검", images: 0 };
/** The turn card's facts for the request cases: the raw request, its summary and two earlier turns. */
const TURN_VIEW = {
  request: { text: "ㅇㅇㅇㅇ 그렇게 해. 읽을 것으로 두긴 해야할 것 같긴 한데?\n그 외에 뭔가 상태 애매한 거 있는지 점검들 좀 해줘", cut: false, images: 0, at_unix_ms: Date.now() - 180_000, sender: { kind: "operator" as const } },
  request_summary: TURN_REQUEST.text,
  recent_turns: [
    { request: "완료 후 흰 원 표시 방식 제안", end: "done" as const, line: "두 가지 안을 정리함", at_unix_ms: Date.now() - 720_000 },
    { request: "미확인 상태 원인 조사", end: "done" as const, line: null, at_unix_ms: Date.now() - 1_860_000 },
  ],
  later_by: null,
};
const HEADERS: [string, Omit<PaneHeader, "working">][] = [
  ["sleeping", { ...NO_NOTICE, notice: { kind: "sleeping", tone: "muted", reason: "12분 동안 휴면 중", since_unix_ms: null, exit_code: null } }],
  ["exit", { ...NO_NOTICE, notice: { kind: "exit", tone: "error", reason: null, since_unix_ms: null, exit_code: 1 } }],
  ["device", { ...NO_NOTICE, notice: { kind: "device_offline", tone: "muted", reason: "mini", since_unix_ms: null, exit_code: null } }],
  ["approval", { ...NO_NOTICE, band: { kind: "approval", reason: null, since_unix_ms: null, read: false, action: null, more: 0 } }],
  ["answer", { ...NO_NOTICE, band: { kind: "answer", reason: "기존 stdin 종료 경로도 남길까요?", since_unix_ms: null, read: true, action: null, more: 0 } }],
  ["blocked", { ...NO_NOTICE, band: { kind: "blocked", reason: "gh 로그인이 만료돼 PR을 못 만듦", since_unix_ms: null, read: false, action: null, more: 0 } }],
  ["fix", NO_NOTICE],
  ["merge", NO_NOTICE],
  ["raised", { ...NO_NOTICE, band: { kind: "raised", reason: null, since_unix_ms: null, read: false, action: { pane_id: "a1c1", label: RAISED.title }, more: 1, raised: RAISED } }],
  ["working", NO_NOTICE], ["idle", NO_NOTICE], ["children", NO_NOTICE], ["unknown", NO_NOTICE],
];
// The twelve approved pane-header situations use the production header and
// popover components over explicit wire fixtures. The other examples above
// remain for the Sessions panel's unrelated fixture.
const PANE_HEADER_CASES: [string, string, Omit<PaneHeader, "working">][] = [
  ["progress", "01 · 진행 중", { ...NO_NOTICE, progress: "상황별 헤더 시안을 정리하고 있어요" }],
  ["question", "02 · 답변이 필요함", { ...NO_NOTICE, band: { kind: "answer", reason: "그룹 이름은 직접 정하게 할까요?", since_unix_ms: null, read: false, action: null, more: 0 } }],
  ["approval-case", "03 · 승인이 필요함", { ...NO_NOTICE, band: { kind: "approval", reason: "권한 승인을 기다리고 있어요", since_unix_ms: null, read: false, action: null, more: 0 } }],
  ["blocked-case", "04 · 막힘", { ...NO_NOTICE, band: { kind: "blocked", reason: "접근 권한을 확인해 주세요", since_unix_ms: null, read: false, action: null, more: 0 } }],
  ["raised-case", "05 · 자식에게 사용자 승인이 필요함", { ...NO_NOTICE, band: { kind: "raised", reason: null, since_unix_ms: null, read: false, action: { pane_id: PANE_RAISED.open_pane_id, label: PANE_RAISED.title }, more: 0, raised: PANE_RAISED } }],
  ["stopped", "06 · 일이 남음", { ...NO_NOTICE, band: { kind: "stopped", reason: "응답이 중단됐어요", since_unix_ms: null, read: false, action: null, more: 0 } }],
  ["solo", "07 · 자식 없이 진행 중", { ...NO_NOTICE, progress: "상황별 헤더 시안을 정리하고 있어요" }],
  ["tree-only", "08 · progress 없음 · 자식 있음", NO_NOTICE],
  ["empty", "09 · progress도 자식도 없음", NO_NOTICE],
  ["finished", "10 · 작업 완료 · 자식 있음", NO_NOTICE],
  ["popover", "11 · 자식 목록 열기", { ...NO_NOTICE, progress: "검토 결과를 정리하고 있어요" }],
  ["narrow", "12 · 좁은 패널 · 긴 progress", { ...NO_NOTICE, progress: "헤더 상태별 표시와 요청 우선순위를 검토하고 있어요" }],
  // PRD pane-last-request: the row leads with the turn's request (state=turn).
  ["turn-question", "13 · 요청 → 질문", { ...NO_NOTICE, request: TURN_REQUEST, band: { kind: "answer", reason: "1~3번을 한 PR로 묶어 진행할지 답해 주세요.", since_unix_ms: null, read: false, action: null, more: 0 } }],
  ["turn-working", "14 · 요청 → 작업 중", { ...NO_NOTICE, request: { text: "탭을 넘는 종합 뷰 + AI 작업 구상", images: 0 }, progress: "종합 뷰의 구성 요소를 고민하고 있어요" }],
  ["turn-finished", "15 · 끝났는데 아직 안 봄", { ...NO_NOTICE, request: { text: "원격 조회 3초로 바꿔 PR·머지", images: 0 }, finished: true, progress: "원격 조회 주기를 3초로 바꾸고 머지함" }],
  ["turn-narrow", "16 · 좁은 pane · 질문", { ...NO_NOTICE, request: TURN_REQUEST, band: { kind: "answer", reason: "1~3번을 한 PR로 묶어 진행할지 답해 주세요.", since_unix_ms: null, read: false, action: null, more: 0 } }],
  ["turn-image", "17 · 이미지만 보낸 요청", { ...NO_NOTICE, request: { text: null, images: 2 }, progress: "보낸 화면의 여백을 맞추고 있어요" }],
  ["turn-children", "18 · 요청 → 자식 기다림", { ...NO_NOTICE, request: { text: "요청 추출 누락 원인 찾아줘", images: 0 }, waiting_on_children: 2 }],
  ["turn-request-only", "19 · 작업 중 · 요청 말고 새 사실 없음", { ...NO_NOTICE, request: { text: "OS별 사용자 실패 경험과 개발 불편 우선순위 정리", images: 0 } }],
  // A child pane's header leads with its parent's name, and drops it below the pane breakpoint (state=child).
  ["child-wide", "20 · 자식 pane · 부모 이름", { ...NO_NOTICE, progress: "상황별 헤더 시안을 정리하고 있어요" }],
  ["child-narrow", "21 · 좁은 자식 pane · 화살표만", { ...NO_NOTICE, progress: "상황별 헤더 시안을 정리하고 있어요" }],
];
const CHILD_CASES = ["child-wide", "child-narrow"];
const TURN_CASES = ["turn-question", "turn-working", "turn-finished", "turn-narrow", "turn-image", "turn-children", "turn-request-only"];

// Frozen visual examples, not a second implementation of core judgments.
const MARKS: [RowMark, StatusTone["kind"], RequestVerb][] = [
  ["turn", "warning", "answer"], ["turn", "warning", "answer"], ["turn", "warning", "answer"],
  ["done", "success", "review"], ["done", "success", "review"], ["going", "working", "working"],
  ["going", "working", "waiting"], ["quiet", "subtle", "idle"], ["quiet", "subtle", "result"],
  ["turn", "warning", "answer"], ["turn", "warning", "answer"], ["waiting", "working", "waiting"], ["waiting", "working", "waiting"],
];

export function SessionWorkflowScene({ scene: kind, theme, content, scale, width, state }: SceneParams & { scene: "session-panel" | "pane-header" }) {
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
        wait: example.wait, descendant_counts: example.wait === "children" ? { error: 0, approval: 0, question: 0, working: 2, done: 0 } : undefined,
        state: { ...seed.state, verb, mark_tone: { kind: tone, read: example.read ?? false }, chip_tone: { kind: tone, read: example.read ?? false }, session: { group: example.group, line: example.line, unfinished: example.unfinished ?? false }, ask: example.ask ?? null, tree_rank: 1,
          pr: example.pr ? { count: 1, worst: example.pr, pulls: [{ index: 0, state: example.pr }] } : null,
          request_since: Date.now() - 180_000, line: example.wait ? { text: "", mode: "waiting", tone: { kind: "working", read: false }, pull_request: example.pull } : example.line ? { text: example.line, mode: "request", tone: { kind: "subtle", read: false } } : null },
        request: { verb, verb_since_unix_ms: Date.now(), line: example.line ?? undefined, request: null, later_by: null, pull_requests: example.pr ? [{ number: 220 + index, title: example.title, url: `https://example.invalid/pull/${220 + index}`, badge: example.pr === "merged" ? "merged" : "open", checks: example.pr === "failed" ? "failed" : example.pr === "pending" ? "pending" : "passing", review: example.pr === "mergeable" ? "approved" : "review_required", head_branch: "prd/session-ui", closing_issues: [], live: true, duty: true, created: true, settled_at_unix_ms: null }] : [] },
        resolved: example.group === "resolved" ? { at_unix_ms: Date.now(), source: "operator" } : null,
        // A quiet task starred to revisit, and a working one that keeps its group with a ★ (PRD revisit-group B6, B7).
        revisit: index === 6 || index === 4 ? { starred: true, read_ask: false, since_unix_ms: Date.now() - 600_000 } : example.read ? { starred: false, read_ask: true, since_unix_ms: Date.now() - 900_000 } : null,
      };
    });
    const feature = { ...checkout, id: "session-ui", label: "prd/session-ui", branch: "prd/session-ui" };
    const scope = { ...project.agent_scope, members: agents.map((agent, index) => ({ pane_id: agent.pane_id, project_id: project.id, checkout_id: index === 5 ? checkout.id : feature.id })), work: { s3: { pull: 0, more: 0, issues: [], issue_chips: [] }, s4: { pull: 0, more: 0, issues: [], issue_chips: [] }, s5: { pull: null, more: 0, issues: ["issue-219"], issue_chips: ["issue-219"] }, s6: { pull: 0, more: 0, issues: [], issue_chips: [] } }, tasks: { cards: TASKS.map(([member, state]) => ({ root: agents[member]!.pane_id, state, members: [agents[member]!.pane_id], checkouts: [member === 5 ? checkout.id : feature.id], recency: "", ...(EXAMPLES[member]!.read ? { read: true as const } : {}) })), needs_you: 5, recency: "" } };
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
        request: { verb: index === 0 ? "working" : "idle", verb_since_unix_ms: Date.now(), request: null, later_by: null, pull_requests: [{ number, title: child.identity_label, url: `https://example.invalid/pull/${number}`, badge: "open", checks: state === "failed" ? "failed" : state === "pending" ? "pending" : "passing", review: state === "mergeable" ? "approved" : "review_required", head_branch: "feat/session-row-clarity-and-branch-label", closing_issues: [], live: true, duty: true, created: true, settled_at_unix_ms: null }] } };
    });
    const childPanes = children.map((child) => ({ ...pane, id: child.pane_id, identity_label: child.identity_label }));
    feature.tabs[0]!.panes.push(childPanes[0]!);
    checkout.tabs[0]!.panes.push(...childPanes.slice(1));
    const headers = Object.fromEntries([...HEADERS.map(([id, header]) => [id, { ...header, working: id === "working", band: header.band && { ...header.band, since_unix_ms: Date.now() - 180_000 } }]), ...PANE_HEADER_CASES.map(([id, , header]) => [id, { ...header, progress: id === "narrow" && content === "long" ? "헤더 상태별 표시와 요청 우선순위를 검토하고 다음 단계의 배치와 접근성까지 다시 확인하고 있어요" : header.progress, working: ["progress", "solo", "popover", "narrow", "turn-working", "turn-image", "turn-request-only"].includes(id), band: header.band && { ...header.band, since_unix_ms: Date.now() - 180_000 } }])]);
    // The fix and merge panes own one PR each: their header's PR chip is all
    // that says it, with no second row (B21, B22).
    const ownPr = (id: string): Pick<AgentRow, "state" | "request"> => {
      const state = id === "fix" ? "failed" as const : "mergeable" as const;
      return {
        state: { ...seed.state, pr: { count: 1, worst: state, pulls: [{ index: 0, state }] } },
        request: { verb: "working", verb_since_unix_ms: Date.now(), line: undefined, request: null, later_by: null, pull_requests: [{ number: 221, title: "세션 상태 투영 정리", url: "https://github.com/acme/app/pull/221", badge: "open", checks: state === "failed" ? "failed" : "passing", review: "approved", head_branch: "prd/session-ui", closing_issues: [], live: true, duty: true, created: true, settled_at_unix_ms: null }] },
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
    const raisedChild: AgentRow = { ...children[0]!, id: PANE_RAISED.pane_id, pane_id: PANE_RAISED.pane_id, identity_label: PANE_RAISED.title, lineage_parent_pane_id: "raised-case", lineage_path_pane_ids: ["raised-case", PANE_RAISED.pane_id], lineage_child_pane_ids: [] };
    const popoverChildren: AgentRow[] = ["레이아웃 설계", "구현", "검증"].map((label, index) => ({ ...children[index]!, id: `pane-popover-child-${index}`, pane_id: `pane-popover-child-${index}`, identity_label: label, lineage_parent_pane_id: "popover", lineage_path_pane_ids: ["popover", `pane-popover-child-${index}`], lineage_child_pane_ids: [], mark: index === 2 ? "done" as const : "going" as const, state: { ...children[index]!.state, session: { group: index === 2 ? "done" as const : "working" as const, line: ["상태별 헤더 시안 정리 중", "진행 문구 연결 중", "검토 결과 도착"][index]!, unfinished: false } } }));
    const narrowChildren: AgentRow[] = Array.from({ length: 12 }, (_, index) => {
      const id = `pane-narrow-child-${index}`;
      return { ...childExamples[0]!, id, pane_id: id, identity_label: `자식 ${index + 1}`, lineage_parent_pane_id: "narrow", lineage_path_pane_ids: ["narrow", id], lineage_child_pane_ids: [], state: { ...childExamples[0]!.state, tree_rank: index + 1 } };
    });
    const headerAgents = [...HEADERS.map(([id]) => id), ...PANE_HEADER_CASES.map(([id]) => id)].map((id) => ({ ...seed, id, pane_id: id,
      identity_label: PANE_HEADER_CASES.some(([caseId]) => caseId === id) ? "Group tab 기능 설계" : seed.identity_label,
      device_id: id === "device" ? "mini" : seed.device_id,
      lineage_path_pane_ids: [id],
      descendant_mark: id === "raised" ? { kind: "raised" as const, count: 2 } : id === "raised-case" ? { kind: "raised" as const, count: 1 } : null,
      raised: id === "raised" ? [RAISED, { ...RAISED, raised_pane_id: "a1c2", origin_title: children[1]!.identity_label, pane_id: "a1c2", open_pane_id: "a1c2", title: children[1]!.identity_label, verb: "answer" as const, what: "PR을 함께 올릴까요?" }] : id === "raised-case" ? [PANE_RAISED] : [],
      // A childless approval and merge show the row only for the ask, and none for a PR.
      lineage_child_pane_ids: ["idle", "approval", "merge", "solo", "empty", "turn-question", "turn-working", "turn-finished", "turn-image", ...CHILD_CASES].includes(id) ? [] : id === "raised-case" ? [raisedChild.pane_id, ...childExamples.slice(1, 3).map((child) => child.pane_id)] : id === "popover" ? popoverChildren.map((child) => child.pane_id) : id === "narrow" ? narrowChildren.map((child) => child.pane_id) : ["children", "progress", "question", "approval-case", "blocked-case", "stopped", "tree-only", "finished", "turn-narrow", "turn-children"].includes(id) ? childExamples.slice(0, 3).map((child) => child.pane_id) : id === "unknown" ? ["b2-unknown"] : ["a1c1", "a1c2"],
      direct_child_counts: id === "narrow" ? { error: 0, approval: 0, question: 0, working: 12, done: 0 } : ["children", "progress", "question", "approval-case", "blocked-case", "raised-case", "stopped", "tree-only", "finished", "popover", "turn-narrow"].includes(id) ? { error: 0, approval: 0, question: 1, working: 1, done: 1 } : id === "unknown" ? { error: 0, approval: 0, question: 0, working: 0, done: 0 } : { error: 0, approval: 0, question: 0, working: 1, done: 0 },
      ...(id === "fix" || id === "merge" ? ownPr(id) : {}),
      ...(TURN_CASES.includes(id) ? { session_id: `turn-${id}`, state: { ...seed.state, pr: null }, request: { verb: "answer" as const, verb_since_unix_ms: Date.now(), line: "1~3번을 한 PR로 묶어 진행할지 답해 주세요.", end: "question" as const, pull_requests: [], ...TURN_VIEW } } : {}) }));
    base.rest.terminal = { headers };
    // The Conductor row's mission, its open decision the row asks (needs-you-cleanup B1, B5).
    const conductor = agents[9]!;
    base.rest.missions = { items: [{ id: "m-gallery", title: "사이드바 정리", workspace_id: project.id, project: "/repo", checkout_path: "/repo", device_id: conductor.device_id, status: "running", scope: "pr", cap: 4, exempt: false, strength: null, other_vendor: null, conductor: { pane_id: conductor.pane_id, name: conductor.identity_label, agent_kind: "claude", session: null, present: true, takes_answers: true }, nodes: Array.from({ length: 8 }, (_, index) => ({ key: `n${index}`, title: `n${index}`, kind: "change", depends_on: [], status: "planned" })), merge_order: [], check: null, github: null, decision: "뼈대의 check 줄을 건너뛸까요?", question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: 1, closed_at_unix_ms: null, running: 0, stale: false, github_stale: false, github_read_at_unix_ms: null } as unknown as Mission] } as typeof base.rest.missions;
    base.rest.status = { ...base.rest.status, remote: [{ target_id: "mini", state: "not_connected", message: "Mac mini 연결되지 않음", herdr_version: null, session: null }] };
    return { ...base, agents: [...agents, ...children, ...childExamples, raisedChild, ...popoverChildren, ...narrowChildren, ...headerAgents], pane };
  }, [content]);
  // `turn-two-lines` and `turn-off` draw the turn cases in the View menu's other modes.
  const turnRow = state === "turn-two-lines" ? "two_lines" : state === "turn-off" ? "off" : "one_line";
  const turn = state === "turn" || state === "turn-two-lines" || state === "turn-off";
  useLayoutEffect(() => {
    useShellStore.setState({ rest: { ...fixture.rest, ui_state: { ...fixture.rest.ui_state, pane_turn_row_mode: turnRow } }, agents: fixture.agents, connection: "gone" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
  }, [fixture, theme, scale, turnRow]);
  // state=connection: a copy too old for Hide wears the Update needed badge, and a session started before the update
  // put the piece in says to restart; the other headers keep the integration icon (docs/UI_BEHAVIOR.md).
  if (kind === "pane-header" && state === "connection") {
    const header = (id: string, connection: PaneConnection) => ({ ...fixture.pane, id, identity_label: "Group tab 기능 설계", children: { instrumented: false, uninstrumented_reason: null, uninstrumented_label: null, chips: [], ...fixture.pane.children, connection } });
    const cases: [string, string, string, PaneConnection][] = [
      ["update-agent", "업데이트 필요 · Claude Code 2.1.200", "claude", { connected: false, can_reopen: false, reason: "update_agent", reopen: null, update: { installed: "2.1.200", minimum: "2.1.278", path: "/Users/me/.local/share/claude/versions/2.1.200", command: "claude update" } }],
      ["restart-agent", "업데이트 뒤 · 다시 시작하면 연결", "omp", { connected: false, can_reopen: false, reason: "started_before_hide", reopen: null }],
    ];
    return <TooltipProvider><div style={{ width: width ?? undefined }} className="flex flex-col gap-lg bg-background p-xl text-foreground" data-gallery-scene={kind} data-gallery-state={state}>{cases.map(([id, label, agentKind, connection]) => <div key={id} data-pane-header-case={id} className="min-w-0"><div className="pb-xs text-micro text-muted-foreground">{label}</div><div className="h-[var(--size-pane-state-example)] min-w-0 border border-border"><PaneView pane={header(id, connection)} transport={undefined} focused={false} scale={1} actions={actions} agentKind={agentKind} markSymbol="quiet" markTone="text-muted-foreground" paneCount={2} zoomed={false} /></div></div>)}</div></TooltipProvider>;
  }
  return <TooltipProvider>{kind === "session-panel" ? <div className="flex h-full bg-background text-foreground" data-gallery-scene={kind}><TaskLens actions={actions} /></div> : <div style={{ width: width ?? undefined }} className="flex flex-col gap-lg bg-background p-xl text-foreground" data-gallery-scene={kind} data-gallery-state={state}>{(state === "child" ? PANE_HEADER_CASES.filter(([id]) => CHILD_CASES.includes(id)) : PANE_HEADER_CASES.slice(turn ? 12 : state === "edge" ? 6 : 0, turn ? 12 + TURN_CASES.length : state === "edge" ? 12 : 6)).map(([id, label]) => <div key={id} data-pane-header-case={id} className={id === "narrow" || id === "turn-narrow" || id === "child-narrow" ? "w-(--size-pane-narrow-example) max-w-full min-w-0" : "min-w-0"}><div className="pb-xs text-micro text-muted-foreground">{label}</div><div className="h-[var(--size-pane-state-example)] min-w-0 border border-border"><PaneView pane={{ ...fixture.pane, id, identity_label: CHILD_CASES.includes(id) ? "UI 검증" : content === "long" ? "긴 한국어 제목과 변경 사항을 좁은 화면에서 확인하는 작업" : "Group tab 기능 설계", lineage_path: CHILD_CASES.includes(id) ? [{ pane_id: "progress", label: content === "long" ? "긴 한국어 부모 작업 이름을 좁은 화면에서 확인하는 작업" : "Group tab 기능 설계", siblings: [] }, { pane_id: id, label: "UI 검증", siblings: [] }] : undefined }} transport={undefined} focused={false} scale={1} actions={actions} agentKind="codex" markSymbol={id === "finished" ? "done" : "quiet"} markTone={id === "finished" ? "text-success" : "text-muted-foreground"} paneCount={2} zoomed={false} /></div></div>)}</div>}</TooltipProvider>;
}
