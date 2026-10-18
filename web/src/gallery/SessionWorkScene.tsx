// Production screens over invented wire values for the approved record design.
// These examples prove layout; the isolated daemon tests own runtime policy.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TaskLens } from "../TaskLens";
import { Palette } from "../Palette";
import { SlotSessionRecord } from "../SessionRecord";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import type { MissedWork, SessionRecord as RecordValue, WorkOutput, WorkRequest, WorkSession, WorkTask } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";

const AT = 1_790_000_000_000;
const SESSION: WorkSession = { device_id: "local", agent: "claude", session_id: "record-example" };
const CHILD_SESSION: WorkSession = { ...SESSION, agent: "claude", session_id: "child-example" };
const task = (id: string, chapter: string, name: string, end: WorkTask["end"], done: boolean, at: number): WorkTask => ({ id, chapter_id: chapter, name, end, done, at_unix_ms: at, last_at_unix_ms: at + 1000, unjudged: false });
const request = (offset: number, text: string, task_ids: string[], patch: Partial<WorkRequest> = {}): WorkRequest => ({ offset, text, at_unix_ms: AT + offset * 60_000, author: "operator", task_ids, chapter_id: "chapter-1", judged: true, ...patch });
const missed = (id: string, text: string, turn_offset: number, status: MissedWork["status"]): MissedWork => ({ id, kind: "follow_up", text, status, at_unix_ms: AT, turn_offset, issue_repository: null, issue_number: null, creating: false, backlog_key: null });
const pr = (number: number, title: string, state: string, source: WorkSession, sourceName: string, task_id: string | null): WorkOutput => ({ kind: "pr", repository: "example/hide", number, title, url: `https://example.invalid/pull/${number}`, state, stale: false, last_read_at_unix_ms: AT, created_at_unix_ms: AT + number, task_id, source, source_name: sourceName });
// The approved record: three tasks, two PRs (one a child's), the child under the task that delegated it, the requests at the end.
const BASE: RecordValue = {
  session: SESSION, title: "성능 최적화 리뷰와 서비스 개선안", fallback_title: "Claude", native_title: "성능 최적화 리뷰", manual_title: null, state: "idle", last_end: "unfinished", cwd: "/work/hide", path: null, started_at_unix_ms: AT, ended_at_unix_ms: null,
  chapters: [{ id: "chapter-1", goal: "성능 최적화 리뷰와 서비스 개선안", at_unix_ms: AT }],
  tasks: [
    task("task-1", "chapter-1", "codex 성능 최적화 작업 리뷰 및 서비스 개선안 도출", "done", true, AT),
    task("task-2", "chapter-1", "설치 빌드 실측 및 개선 제안 안전성 평가", "done", true, AT + 120_000),
    task("task-3", "chapter-1", "버그 3건 한 PR로 묶어 수정 (Opus 에이전트)", "waiting", false, AT + 180_000),
  ],
  requests: [
    request(1, "옆 codex pane의 퍼포먼스 최적화 작업을 리뷰하고 그다음에 실제로 우리 서비스에서 느려지거나 할 때 (herdr 병목은 우선 어쩔 수 없음) 어떻게 개선할 수 있을지 고민해보자", ["task-1"]),
    request(2, "설치가 됐으니 이제 너가 직접 하나 hide 실측을 진행해보고 너가 제안한것들 혹시 코드 구조나 앱이 불안정해지는 건 없는지 평가해줘", ["task-2"]),
    request(3, "버그 3건 한 PR로 묶어서 진행해줘 opus 5.5로 하나 띄워서 진행시키자", ["task-3"]),
    request(4, "PR 열리면 리뷰하고 머지까지 진행해줘", ["task-3"]),
    request(5, "그러면 지금 처리하는 작업들 말고 그다음에 너가 해야할것들 정리좀 해줘봐 ~", []),
    request(6, "잘 되고 있어?", []),
  ],
  missed: [missed("missed-1", "Codex developer role 매핑 수정", 2, "resolved"), missed("missed-2", "links ledger 세션 불일치 수정", 2, "resolved")],
  outputs: [pr(1100, "Measure the install build after the fixes", "review", SESSION, "성능 최적화 리뷰와 서비스 개선안", null), pr(1093, "Stop three resident hided loops", "merged", CHILD_SESSION, "상주 데몬 버그 3건 수정 PR", "task-3")],
  children: [{ session: CHILD_SESSION, title: "상주 데몬 버그 3건 수정 PR", fallback_title: "Claude", native_title: "상주 데몬 버그 3건 수정 PR", manual_title: null, state: "idle", last_end: "done", cwd: "/work/hide/task", path: null, started_at_unix_ms: AT + 190_000, ended_at_unix_ms: null, chapters: [{ id: "child-chapter", goal: "상주 데몬 버그 수정", at_unix_ms: AT }], tasks: [task("child-task", "child-chapter", "links ledger 루프 멈추기", "done", true, AT)], requests: [request(1, "links ledger 루프를 멈추는 변경을 만들어줘", ["child-task"], { author: "parent", chapter_id: "child-chapter" })], missed: [], outputs: [], children: [] }],
};

/** The scene's record for a `case`: the default, or one that shows a state the record draws. */
function recordFor(base: RecordValue, kind: string | null): RecordValue {
  const record = structuredClone(base);
  if (kind === "open-missed") record.missed.push(missed("missed-3", "재측정 전 설치본 버전 확인 안 됨", 2, "open"), missed("missed-4", "이슈 생성 실패 뒤 재시도 검증", 99, "open"));
  if (kind === "now") {
    record.state = "working";
    record.requests.push(request(7, "설치하고 재측정해줘", [], { judged: false }));
  }
  if (kind === "goals") {
    record.chapters.push({ id: "chapter-2", goal: "세션 검색과 놓친 일 정리", at_unix_ms: AT + 240_000 });
    record.tasks.push(task("task-4", "chapter-2", "⌘K 세션 검색", "unfinished", false, AT + 260_000));
    record.outputs.push(pr(1122, "Session search in the palette", "ci_failed", SESSION, "성능 최적화 리뷰와 서비스 개선안", "task-4"));
    record.requests.push(request(8, "세션 검색을 연결하고 좁은 칸에서도 읽기 좋게 해줘", ["task-4"], { chapter_id: "chapter-2" }));
  }
  if (kind === "delegated") record.requests = record.requests.map((one) => ({ ...one, author: "parent" as const }));
  if (kind === "empty") { record.outputs = []; record.tasks = []; record.requests = []; record.missed = []; record.children = []; }
  return record;
}

export function SessionWorkScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage("ko");
  const state = params.get("state") ?? "rest";
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const scale = Number(params.get("scale") ?? 1);
  const fixture = useMemo(() => {
    const base = sidebarScene("reference", REFERENCE_FOLDS, AT);
    const project = base.rest.navigator!.workspaces!.find((row) => row.id === "hide")!;
    const record = recordFor(BASE, params.get("case"));
    record.cwd = project.checkouts[0]!.path;
    if (params.get("content") === "long") record.title += " - 긴 한국어 제목과 요청의 줄바꿈 확인";
    base.rest.status = { ...base.rest.status, background_ai: { provider: "claude", providers: [], unavailable_reason: null, enabled: state !== "off", chosen: true, agent_summary: true } };
    base.rest.navigator!.focused_workspace_id = project.id;
    base.rest.navigator!.focused_checkout_id = project.checkouts[0]!.id;
    return { base, record, project };
  }, [params, state]);
  const actions = useMemo(() => createActions((event) => {
    if (event.kind === "session_work_search") useShellStore.setState({ sessionWorkSearch: { query: String(event.payload.query), loading: false, failure: null, results: [{ session: SESSION, title: fixture.record.title, matched: "세션 검색을 연결하고 좁은 칸에서도 읽기 좋게 해줘", at_unix_ms: AT, cwd: fixture.record.cwd }] } });
    return true;
  }), [fixture]);
  useLayoutEffect(() => {
    useShellStore.setState({ rest: fixture.base.rest, agents: fixture.base.agents, connection: "live", linkSummaries: { filling:false, projects:{[fixture.project.id]:{work:[{session:SESSION,title:fixture.record.title,fallback_title:fixture.record.fallback_title,native_title:fixture.record.native_title,manual_title:null,cwd:fixture.record.cwd,ended_at_unix_ms:AT,open_missed:fixture.record.missed.filter((item)=>item.status==="open").length,subtree_open_missed:0,missed:fixture.record.missed}]}} }, sessionRecord: { session: SESSION, loading: state === "loading", failure: state === "failed" ? "fixture_read_failed" : null, record: state === "loading" || state === "failed" ? null : fixture.record } });
    useUiStore.setState({ screen: { kind: "workspace" }, overlay: state === "search" ? "search" : "none" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
  }, [fixture, state, theme, scale, params]);
  return <TooltipProvider><div className="flex h-full min-w-0 flex-col bg-background text-foreground" data-gallery-scene="session-record">
    {state === "ended" ? <TaskLens actions={actions} /> : <SlotSessionRecord actions={actions} target={SESSION} />}
    {state === "search" ? <Palette actions={actions} /> : null}
  </div></TooltipProvider>;
}
