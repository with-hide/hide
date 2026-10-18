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
// Invented content matching the user-approved PR/session cards.
const CHILDREN = [
  ["agent-journey 브리프 실행 및 검증 결과물", "claude", [1372, 1371, 1369, 1368, 1333, 1326, 1370, 1361, 1360, 1357, 1356, 1354, 1335, 1340]],
  ["agent-folder-trust 브리프 작업 수행", "claude", [1306]],
  ["agy CI 쿼터 브리프 수행 결과물", "codex", [1312, 1309]],
  ["데스크톱 E2E 마친 브리프 이행 결과", "opencode", [1324, 1315]],
  ["agy 가용성 probe 실행 결과 보고서", "claude", [1331]],
] as const;
const children: RecordValue[] = CHILDREN.map(([title, agent, numbers], i) => {
  const session = { ...CHILD_SESSION, agent, session_id: `child-${i}` };
  return { session, title, fallback_title: agent, native_title: title, manual_title: null, state: "idle", last_end: i ? "done" : "waiting", cwd: "/work/hide", path: null, started_at_unix_ms: AT + 2, ended_at_unix_ms: null, chapters: [], tasks: [task("child-task", "chapter-1", "에이전트별·OS별 E2E 검증", i ? "done" : "waiting", i > 0, AT)], requests: [request(1, "에이전트별·OS별 E2E 검증을 진행하고 결과를 정리해줘.", ["child-task"], { author: "parent" })], missed: i === 0 ? [missed("child-problem", "Windows 저장 시 백신·색인 잠금에 대한 재시도 장치 추가", 1, "open"), { ...missed("child-suggestion", "실패한 검증을 다시 실행할 명령 함께 표시", 1, "open"), kind: "problem" }] : [], outputs: numbers.map((number, n) => pr(number, "에이전트별·OS별 검증 결과 정리", i === 0 && n < 5 ? "pending" : "merged", session, title, "child-task")), children: [] };
});
const BASE: RecordValue = {
  session: SESSION, title: "릴리스 스킬과 로컬 검증 체계", fallback_title: "Claude Code", native_title: "릴리스 스킬과 로컬 검증 체계", manual_title: null, state: "working", last_end: "working", cwd: "/work/hide", path: null, started_at_unix_ms: AT, ended_at_unix_ms: null,
  chapters: [{ id: "chapter-1", goal: "릴리스 스킬과 로컬 검증 체계", at_unix_ms: AT }],
  tasks: [task("task-1", "chapter-1", "릴리스 절차를 스킬로 정리", "done", true, AT), task("task-2", "chapter-1", "Mac에서 로컬 검증", "working", false, AT + 120_000)],
  requests: [request(1, "릴리스 절차를 재사용할 수 있는 스킬로 정리해줘.", ["task-1"]), request(2, "이 Mac에서 검증하고, 결과와 남은 작업을 정리해줘.", ["task-2"]), request(3, "진행 상황과 확인이 필요한 점을 알려줘.", [])],
  missed: [{ ...missed("missed-1", "릴리스 검증 실패 시 멈춘 단계를 알기 어려움", 2, "open"), kind: "problem" }, missed("missed-2", "검증 결과에 OS·에이전트별 통과 여부 표시", 2, "open"), missed("missed-3", "실패한 검증을 다시 실행할 명령 함께 표시", 2, "open")],
  outputs: [pr(1365, "릴리스 스킬과 로컬 검증 체계", "merged", SESSION, "릴리스 스킬과 로컬 검증 체계", "task-1"), ...children.flatMap((child) => child.outputs)],
  children,
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
    if (state === "off" || state === "pending" || state === "question") {
      record.title = "";
      record.tasks = [];
      record.missed = [];
      record.requests = [request(1, state === "question" ? "이 프로젝트 구조가 어떻게 되어 있어?" : "이 Mac에서 검증하고 결과를 정리해줘.", [], { judged: state === "question" })];
    }
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
  return <TooltipProvider><div style={{ width: Number(params.get("width") ?? 680) }} className="flex h-full min-w-0 flex-col bg-background text-foreground" data-gallery-scene="session-record">
    {state === "ended" ? <TaskLens actions={actions} /> : <SlotSessionRecord actions={actions} target={SESSION} />}
    {state === "search" ? <Palette actions={actions} /> : null}
  </div></TooltipProvider>;
}
