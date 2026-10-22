// Production screens over invented wire values for the approved record design.
// These examples prove layout; the isolated daemon tests own runtime policy.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { legacyAgentRow } from "../../test/legacyAgentRow";
import { requireInterfaceLanguage } from "../i18n/locale";
import { TaskLens } from "../TaskLens";
import { Palette } from "../Palette";
import { SlotSessionRecord } from "../SessionRecord";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import type { MissedWork, SessionRecord as RecordValue, WorkFlow, WorkOutput, WorkRequest, WorkSession } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";

const AT = new Date(2026, 9, 22, 11, 4).getTime();
const MIN = 60_000;
const SESSION: WorkSession = { device_id: "local", agent: "claude", session_id: "record-example" };
const CHILD_SESSION: WorkSession = { ...SESSION, agent: "claude", session_id: "child-example" };
const request = (offset: number, minutes: number, text: string, patch: Partial<WorkRequest> = {}): WorkRequest => ({ offset, text, at_unix_ms: AT + minutes * MIN, author: "operator", images: 0, summary: null, answers: null, letter_from: null, count: 1, goal_command: false, ...patch });
const ask = (id: string, text: string): MissedWork => ({ id, kind: "ask", text, status: "open", at_unix_ms: AT + 230 * MIN, turn_offset: 9, issue_repository: null, issue_number: null, creating: false, backlog_key: null });
const pr = (number: number, title: string, state: string, source: WorkSession, sourceName: string): WorkOutput => ({ kind: "pr", repository: "example/hide", number, title, url: `https://example.invalid/pull/${number}`, state, stale: false, last_read_at_unix_ms: AT, created_at_unix_ms: AT + number, source, source_name: sourceName });
const flow = (id: string, name: string, minutes: number, requests: WorkRequest[], result: WorkFlow["result"]): WorkFlow => ({ id, name, at_unix_ms: AT + minutes * MIN, requests, request_count: requests.filter((one) => !one.letter_from).length, result });
const empty = { status: null, asks: [], flows: [], omitted_requests: 0, outputs: [], pr_groups: [], artifacts: [], children: [] } satisfies Partial<RecordValue>;
// Invented content matching the user-approved record design.
const CHILDREN = [
  ["기록 화면 조사", "claude", "done", []],
  ["원장·라벨 조사", "codex", "done", []],
  ["기록 화면 구현", "claude", "working", [[1544, "pending"], [1541, "merged"]]],
] as const;
const children: RecordValue[] = CHILDREN.map(([title, agent, end, prs], i) => {
  const session = { ...CHILD_SESSION, agent, session_id: `child-${i}` };
  const outputs = prs.map(([number, state]) => pr(number, `${title} 결과`, state, session, title));
  return {
    ...empty, session, title, fallback_title: agent, native_title: title, manual_title: null, state: end === "working" ? "working" : "idle", cwd: "/work/hide", path: null, started_at_unix_ms: AT + 200 * MIN, ended_at_unix_ms: null,
    status: { end, line: end === "done" ? "조사 결과를 편지로 보냄" : "기록 화면 구현 중", at_unix_ms: AT + 240 * MIN, limited: null, settled: null },
    flows: [flow(`child-flow-${i}`, title, 200, [request(1, 200, `${title}을 맡아 결과를 편지로 보내줘.`, { author: "parent", summary: `${title} 맡김` })], null)],
    outputs, pr_groups: outputs.length ? [{ child: null, name: title, outputs }] : [],
  };
});
const BASE: RecordValue = {
  ...empty,
  session: SESSION, title: "세션 기록 재설계 PRD", fallback_title: "Claude Code", native_title: "세션 기록 재설계", manual_title: null, state: "idle", cwd: "/work/hide", path: null, started_at_unix_ms: AT, ended_at_unix_ms: null,
  status: { end: "question", line: "PRD 승인할지 골라주세요", at_unix_ms: AT + 240 * MIN, limited: null, settled: null },
  asks: [ask("ask-1", "PRD 승인 여부"), ask("ask-2", "\"설치 안 됨\" 표시를 넣을지")],
  flows: [
    flow("flow-1", "pane 카드 정리", 0, [request(1, 0, "pane 맨 위 이름에 마우스를 올리면 계속 팝오버가 나와서 불편해", { summary: "pane 이름 hover 팝오버 없애기" }), request(2, 8, "두 번째 줄은 지금 상태만 보이게 해줘", { summary: "두 번째 줄은 지금 상태만" }), request(3, 21, "1번 ㅇㅇ 2번은 그럼 어떻게 바뀌는 거야?", { summary: "1번 안 채택, 2번 안이 어떻게 바뀌는지 질문", answers: "두 안 중 고르기" }), request(4, 30, "머지해~", { summary: "PR 머지" })], { kind: "turn", end: "done", line: "PR 1533 머지, 제목 hover 카드 제거", at_unix_ms: AT + 40 * MIN }),
    flow("flow-2", "요청한 작업 체크리스트 분석", 46, [request(5, 46, "요청한 작업이 너무 믿음이 안 가. 세션 10개쯤 보고 뭐가 문제인지 봐줘", { summary: "체크리스트 신뢰 문제를 세션 10개로 분석" }), request(6, 80, "케이스별로 정리해줘", { summary: "케이스별 정리" })], { kind: "turn", end: "done", line: "세션 11개 검증, 수정안 정리", at_unix_ms: AT + 160 * MIN }),
    flow("flow-3", "세션 기록 재설계 PRD", 230, [
      request(7, 217, "케이스별로 시각화해서 보여줘", { summary: "케이스별로 시각화해 보여달라" }),
      request(8, 227, "수정안이 뭔지 설명해줘", { summary: "수정안이 무엇인지 설명해 달라" }),
      request(9, 230, "ㅇㅇ PRD 써줘", { summary: "PRD 쓰기", answers: "PRD로 갈지" }),
      request(10, 238, "🦉 letter from record-research", { letter_from: "기록 화면 조사" }),
      request(11, 266, "바뀌는 것 목업으로 정리해줘", { summary: "바뀌는 것 목업으로 정리", count: 2 }),
      request(12, 267, "이거 봐줘", { summary: "이 화면 확인", images: 1 }),
    ], null),
  ],
  artifacts: ["session-record-goal-flow PRD", "record-revised-cases.html"],
};
BASE.outputs = [pr(1533, "pane 카드 정리", "merged", SESSION, BASE.title), ...children.flatMap((child) => child.outputs)];
BASE.pr_groups = [{ child: null, name: BASE.title, outputs: [BASE.outputs[0]!] }, ...children.filter((child) => child.outputs.length).map((child) => ({ child: child.session, name: child.title, outputs: child.outputs }))];
BASE.children = children;

/** The scene's record for a `case`: the default, or one that shows a state the record draws. */
function recordFor(base: RecordValue, kind: string | null): RecordValue {
  const record = structuredClone(base);
  if (kind === "limited") record.status = { end: "question", line: "릴리스 리허설만 남음", at_unix_ms: AT + 240 * MIN, limited: "한도 소진 · 10-22 13:00 풀림", settled: null };
  if (kind === "all-limited") record.status = { end: "blocked", line: "사용 한도 도달", at_unix_ms: AT + 240 * MIN, limited: "사용 한도 도달", settled: null };
  if (kind === "goal") record.flows.at(-1)!.requests.unshift(request(13, 229, "출시 전 검수 리포트 정리", { goal_command: true, summary: "출시 전 검수 리포트 정리" }));
  if (kind === "legacy") record.flows = record.flows.map((one) => ({ ...one, requests: one.requests.map((row) => ({ ...row, summary: null, answers: null })), result: one.result ? { ...one.result, line: "" } : null }));
  if (kind === "facts") {
    // Hide's facts over the labels' lines (fact-status B1-B5): a merged PR, a wait moved on from, a closing line.
    record.flows[0]!.result = { kind: "settled", state: "merged", numbers: [1533], at_unix_ms: AT + 44 * MIN };
    record.flows.splice(1, 0, flow("flow-1b", "체크리스트 개선 인터뷰", 42, [request(14, 42, "체크리스트를 어떻게 고칠지 인터뷰해줘", { summary: "체크리스트 개선 인터뷰" })], { kind: "moved_on", line: "Q2 범위를 정해 주세요", at_unix_ms: AT + 45 * MIN }));
    record.flows[2]!.result = { kind: "closing", line: "세션 11개 검증, 흐름 묶기 안으로 결정", at_unix_ms: AT + 230 * MIN };
    record.status = { end: "waiting", line: "PR CI 기다리는 중", at_unix_ms: AT + 240 * MIN, limited: null, settled: { state: "merged", numbers: [1548], at_unix_ms: AT + 262 * MIN } };
    record.asks = [];
  }
  if (kind === "omitted") record.omitted_requests = 37;
  if (kind === "delegated") record.flows = record.flows.map((one) => ({ ...one, requests: one.requests.map((row) => ({ ...row, author: "parent" as const })) }));
  if (kind === "empty") Object.assign(record, { ...empty, status: null });
  return record;
}

export function SessionWorkScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const state = params.get("state") ?? "rest";
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const scale = Number(params.get("scale") ?? 1);
  const fixture = useMemo(() => {
    const base = sidebarScene("reference", REFERENCE_FOLDS, AT);
    const project = base.rest.navigator!.workspaces!.find((row) => row.id === "hide")!;
    const record = recordFor(BASE, params.get("case"));
    project.tasks = { source: { kind: "local", label: "Local", name: null, reading: false, failure: null, last_read_at_unix_ms: AT }, tasks: [], overflow: false };
    record.cwd = project.checkouts[0]!.path;
    if (params.get("content") === "long") record.title += " - 긴 한국어 제목과 요청의 줄바꿈 확인";
    base.rest.status = { ...base.rest.status, background_ai: { provider: "claude", providers: [], unavailable_reason: null, enabled: state !== "off", chosen: true, agent_summary: true } };
    base.rest.navigator!.focused_workspace_id = project.id;
    base.rest.navigator!.focused_checkout_id = project.checkouts[0]!.id;
    base.agents = [legacyAgentRow({ ...base.agents.find((row) => row.pane_id === "a1")!, pane_id: "a1", agent_kind: "claude", session_id: SESSION.session_id, identity_label: record.title!, group: "working", mark: "going", activity: "working", status_code: "working", detail: "Mac 로컬 검증 결과와 남은 항목을 정리하고 있어요.", message: "Mac 로컬 검증 결과와 남은 항목을 정리하고 있어요.", changed_at_unix_ms: Date.now() - 60_000 })];
    // A stopped pane, so the record's last real turn is the state on top (B10).
    if (params.get("case")?.endsWith("limited") || params.get("case") === "facts") base.agents = base.agents.map((row) => legacyAgentRow({ ...row, status_code: "done", mark: "done", group: "idle", activity: "stopped" }));
    if (state === "off") {
      // Summaries off: the record keeps no flow names, results or summaries (B28).
      record.asks = [];
      record.artifacts = [];
      record.flows = record.flows.map((one) => ({ ...one, requests: one.requests.map((row) => ({ ...row, summary: null, answers: null })) }));
    }
    if (state.startsWith("restore")) {
      record.path = state === "restore-missing" ? null : "/fixture/conversation.jsonl";
      record.ended_at_unix_ms = AT;
      record.state = "idle";
      record.asks = []; record.children = [];
      base.agents = state === "restore-live" ? base.agents.map(row => legacyAgentRow({...row,status_code:"done",mark:"done",group:"idle",activity:"stopped"})) : [];
    }
    return { base, record, project };
  }, [params, state]);
  const actions = useMemo(() => createActions((event) => {
    if (event.kind === "agent_start_in_checkout") {
      if (state === "restore-failed") useShellStore.setState(s => ({rest:{...s.rest,status:{...s.rest?.status,last_error:{kind:"fixture_refusal",retryable:true,occurred_at:AT,request_id:String(event.payload.request_id),message:"Fixture start refused"}}}}));
      if (state === "restore-unknown") useShellStore.setState(s => ({ rest: { ...s.rest, task_operation: { id: 42, kind: "agent_start", phase: "ready", repository_root: fixture.record.cwd, branch: null, base_branch: null, path: fixture.record.cwd, pane_id: "resumed", agent_kind: "claude", message: null, agent_phase: "unknown", agent_message: null, request_id: String(event.payload.request_id) } } }));
    }
    if (event.kind === "session_work_search") useShellStore.setState({ sessionWorkSearch: { query: String(event.payload.query), loading: false, failure: null, results: [{ session: SESSION, title: fixture.record.title, matched: "세션 검색을 연결하고 좁은 칸에서도 읽기 좋게 해줘", at_unix_ms: AT, cwd: fixture.record.cwd }] } });
    return true;
  }), [fixture, state]);
  useLayoutEffect(() => {
    useShellStore.setState({ rest: fixture.base.rest, agents: fixture.base.agents, connection: "live", linkSummaries: { filling:false, projects:{[fixture.project.id]:{work:[{session:SESSION,title:fixture.record.title,fallback_title:fixture.record.fallback_title,native_title:fixture.record.native_title,manual_title:null,cwd:fixture.record.cwd,ended_at_unix_ms:AT,open_missed:0,subtree_open_missed:0,missed:[]}]}} }, sessionRecord: { session: SESSION, loading: state === "loading", failure: state === "failed" ? "fixture_read_failed" : null, record: state === "loading" || state === "failed" ? null : fixture.record } });
    useUiStore.setState({ screen: { kind: "workspace" }, overlay: state === "search" ? "search" : "none" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
  }, [fixture, state, theme, scale, params]);
  return <TooltipProvider><div style={{ width: Number(params.get("width") ?? 680) }} className="flex h-full min-w-0 flex-col bg-background text-foreground" data-gallery-scene="session-record">
    {state === "ended" ? <TaskLens actions={actions} /> : <SlotSessionRecord actions={actions} target={SESSION} />}
    {state === "search" ? <Palette actions={actions} /> : null}
  </div></TooltipProvider>;
}
