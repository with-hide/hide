// The device Overview (PRD overview-device-tasks; PRD sidebar-task-lens D-20)
// in the production screens over invented device data
// (`overviewSceneData.ts`): the backlog with ⌘⇧M open, Projects after the
// disk sweep, a session's record beside the tabs at Home, and an agent pane
// whose head showed its record in the lane's 보기 slot. No daemon runs; the
// isolated daemon tests own runtime policy.
import { useEffect, useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { OverviewPage } from "../Overview";
import { QuickAdd } from "../QuickAdd";
import { useShellStore } from "../store";
import { PALETTE_HIDE_CLASS } from "../theme";
import { TitleRow } from "../TitleRow";
import { OVERVIEW_ENTRY, useUiStore } from "../ui";
import { WorkspaceScreen } from "../WorkspaceScreen";
import { hideKey, overviewFocusWorld, overviewWorld } from "./overviewSceneData";
import { AGENT_ADAPTERS } from "../agentAdapters";
import { ledgerScene } from "./usageHistorySceneData";
import type { SessionRecord, SessionSearchOptions, SessionSearchStatus, WorkSession } from "../snapshot";
import type { NodeUsage } from "../generated/hided-ws";
import { workKey } from "../sessionWork";
import { installKeyboard } from "../keyboard";
import { Palette } from "../Palette";
import { requireInterfaceLanguage, type InterfaceLanguage } from "../i18n/locale";

const STATES = ["work", "focus", "backlog", "backlog-add", "backlog-detail", "projects", "history", "usage", "home-record", "record"] as const;
type OverviewSceneState = (typeof STATES)[number];
export type OverviewSceneParams = { state: OverviewSceneState; theme: "light" | "dark"; palette: "hide" | "neutral"; language: InterfaceLanguage; scale: number; content?: string };

export function overviewSceneParams(params: URLSearchParams): OverviewSceneParams {
  const state = params.get("state") ?? "work";
  if (!(STATES as readonly string[]).includes(state)) throw new Error(`Unknown overview scene state ${state}`);
  const palette = params.get("palette") ?? "hide";
  if (palette !== "hide" && palette !== "neutral") throw new Error(`Unknown overview scene palette ${palette}`);
  const language = requireInterfaceLanguage(params.get("lang") ?? "ko");
  const scale = Number(params.get("scale") ?? "1");
  if (!Number.isFinite(scale) || scale <= 0) throw new Error("Overview scene scale must be a positive number");
  return { state: state as OverviewSceneState, theme: params.get("theme") === "light" ? "light" : "dark", palette, language, scale, content: params.get("content") ?? "reference" };
}

export function OverviewScene({ state, theme, palette, language, scale, content = "reference" }: OverviewSceneParams) {
  useFollowInterfaceLanguage(language);
  const world = useMemo(() => state === "focus" ? overviewFocusWorld(Date.now(), content) : overviewWorld(Date.now()), [state, content]);
  const history = useMemo(() => [
    { record: world.record, project: "/work/hide", status: "working" as SessionSearchStatus },
    ...[
      { title: "결제 재시도 흐름 정리", requests: ["실패한 결제를 다시 시도할 때 현재 상태를 확인해줘", "중복 결제 방지 조건도 함께 검토해줘"] },
      { title: "혼합 언어 목록 검토", requests: ["한글과 English 제목이 함께 있는 목록의 줄바꿈을 확인해줘", "작은 창에서도 담당자 이름이 읽히는지 검토해줘"] },
      { title: "프로젝트 설정 접근성 확인", requests: ["프로젝트 설정을 키보드로 열고 닫는 흐름을 확인해줘", "설정을 닫으면 원래 버튼으로 포커스를 돌려줘"] },
    ].map(({ title, requests }, index) => {
      const shift = (index + 1) * 86_400_000;
      return {
        project: index === 0 ? "/work/billing-api" : "/work/hide",
        status: "archived" as SessionSearchStatus,
        record: { ...world.record, session: { device_id: "local", agent: index === 0 ? "codex" : "claude", session_id: `gallery-history-${index}` }, title, native_title: title, state: "idle", status: { end: "done", line: `${title} 끝`, at_unix_ms: world.record.started_at_unix_ms! - shift + 30 * 60_000, limited: null }, cwd: index === 0 ? "/work/billing-api" : "/work/hide", started_at_unix_ms: world.record.started_at_unix_ms! - shift, ended_at_unix_ms: world.record.started_at_unix_ms! - shift + 30 * 60_000, asks: [], children: [], outputs: [], pr_groups: [], artifacts: [], flows: [{ ...world.record.flows[0]!, name: title, at_unix_ms: world.record.flows[0]!.at_unix_ms - shift, requests: world.record.flows[0]!.requests.slice(0, 2).map((request, requestIndex) => ({ ...request, at_unix_ms: request.at_unix_ms - shift, text: requests[requestIndex]!, summary: null })), request_count: 2 }] } satisfies SessionRecord,
      };
    }),
  ], [world]);
  // This adapter only serves invented gallery data. Runtime policy is tested
  // against the isolated daemon; no external write is made by this scene.
  const actions = useMemo(() => createActions((event) => {
    const payload = event.payload as { workspace_id?: string; task_key?: string; query?: string; options?: SessionSearchOptions; session?: WorkSession };
    if (event.kind === "issue_detail_request") {
      const workspace = world.rest.navigator?.workspaces?.find((row) => row.id === payload.workspace_id);
      const task = workspace?.tasks?.tasks.find((row) => row.key === payload.task_key);
      if (workspace && task) useShellStore.setState((store) => ({ rest: { ...store.rest, issue_work: { ...store.rest!.issue_work!, detail: {
        workspace_id: workspace.id, device_id: workspace.device_id ?? "local", task_key: task.key, phase: "ready", body: "## 목표\n\n긴 한국어와 English issue titles를 같은 목록에서 읽을 수 있어야 합니다.\n\n- [x] 제목과 메타데이터 비교\n- [ ] 좁은 화면에서 키보드 검토\n\n```ts\nconst visible = items.filter(matches);\n```\n\n### 검토 사항\n\n필터를 바꾼 뒤에도 원래 항목과 위치로 돌아옵니다.", message: null,
        labels: task.labels ?? [], author: task.author ?? "jules", created_at_unix_ms: task.created_at_unix_ms ?? null,
        assignees: task.assignees ?? [], comment_count: 2, comments_cursor: null, comments_has_more: false, comments_limited: false, comments_loading: false,
        comments: [{ author: "mira", created_at_unix_ms: Date.now() - 60_000, body: "**검토 완료**: 본문과 활동을 함께 읽을 수 있습니다." }, { author: "jules", created_at_unix_ms: Date.now(), body: "키보드와 좁은 화면도 확인하겠습니다." }],
      } } } }));
    }
    if (event.kind === "session_work_search") {
      const query = payload.query ?? "";
      const options = payload.options;
      const results = history.flatMap(({ record, project, status }) => {
        if ((options?.project_root && options.project_root !== project) || (options?.agent && options.agent !== record.session.agent) || (options?.status && options.status !== status && !(options.status === "current" && status === "working"))) return [];
        const recorded = record.flows.flatMap((flow) => flow.requests);
        const matches = recorded.filter((request) => (!options?.since_unix_ms || request.at_unix_ms >= options.since_unix_ms) && request.text.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())).map((request) => ({ kind: "request" as const, text: request.text, at_unix_ms: request.at_unix_ms, request_offset: request.offset }));
        return matches.length ? [{ session: record.session, title: record.title, matched: matches[0]!.text, at_unix_ms: matches[0]!.at_unix_ms, cwd: record.cwd, project_root: project, status, matches: matches.slice(0, 5), match_count: matches.length, request_count: recorded.length }] : [];
      });
      useShellStore.setState({ sessionWorkSearch: { query, options, loading: false, failure: null, next_cursor: null, results } });
    }
    if (event.kind === "session_record_open" && payload.session) {
      const record = history.find((row) => workKey(row.record.session) === workKey(payload.session!))?.record ?? null;
      useShellStore.setState({ sessionRecord: { session: payload.session, loading: false, failure: null, record } });
    }
    return true;
  }), [world, history]);
  useEffect(() => installKeyboard(actions), [actions]);
  useLayoutEffect(() => {
    document.documentElement.style.setProperty("--interface-scale", String(scale));
    return () => { document.documentElement.style.removeProperty("--interface-scale"); };
  }, [scale]);
  useLayoutEffect(() => {
    const rest = structuredClone(world.rest);
    const sessionRecord = { session: world.recordSession, loading: false, failure: null, record: world.record };
    if (state === "record") {
      const project = rest.navigator!.workspaces!.find((row) => row.id === "hide")!;
      const checkout = project.checkouts.find((row) => row.id === "hide:sidebar")!;
      rest.navigator!.focused_workspace_id = project.id;
      rest.navigator!.focused_checkout_id = checkout.id;
      const tab = checkout.tabs[0]!;
      const tabId = tab.id;
      if (!tabId) throw new Error("the record scene's checkout has no tab id");
      checkout.strip = [{ id: `strip:${tabId}`, kind: "herdr", source_id: tabId, label: tab.label ?? tabId, preview: false }];
      rest.pane_layouts = [{
        workspace_id: project.id, tab_id: tabId, focused_pane_id: "h-a1", zoomed: false,
        root: { type: "split", direction: "right", ratio: 0.55, first: { type: "pane", pane_id: "h-a1" }, second: { type: "split", direction: "down", ratio: 0.5, first: { type: "pane", pane_id: "h-a1c1" }, second: { type: "pane", pane_id: "h-a1c2" } } },
      }];
      rest.terminal = { headers: Object.fromEntries(checkout.tabs[0]!.panes.map((pane) => [pane.id, { working: pane.id !== "h-a1c2", notice: null, band: null }])) } as never;
      rest.workspace_view = {
        device_id: "local", path: checkout.path, tool: null, last_tool: "prs", prs: { only_checkout: false }, pinned: true, tree: false, lane_width: null, panel_width: null, tree_width: null, lane_called: 0, lane_calls: 0,
        layout: { root: { area: { id: "a1", active: null, displays: [] } }, active_area: "a1", limits: { areas: 6, depth: 3, displays: 32 }, display_count: 0 },
        agent_layout: { root: { area: { id: "g1", active: tabId, displays: [{ id: tabId }] } }, active_area: "g1", canvases: {}, limits: { areas: 6, depth: 3, displays: 16 }, display_count: 1 },
      } as never;
      rest.ui_state = { ...rest.ui_state, detail: { items: [{ kind: "record", session: world.recordSession }], shown: true } } as never;
    }
    const usage: NodeUsage = ledgerScene({ node: "local", agents: AGENT_ADAPTERS.map((adapter) => ({ agent: adapter.herdr_kind, limits: { state: "unsupported" }, consumption: { state: "unsupported" } })), history: null }, "reference");
    if (usage.history) usage.history.models = usage.agents.flatMap((row) => row.consumption.state === "reading" ? row.consumption.value.year.days.map((day) => ({ date: day.date, agent: row.agent, model: row.agent === "claude" ? "model-a" : "model-c", hide_ai: false, amount: day.amount })) : []);
    useShellStore.setState({ rest, usage: { nodes: [usage] }, agents: world.agents, focusedPaneId: world.recordPane, linkSummaries: world.linkSummaries, connection: "live", sessionRecord });
    useUiStore.setState({
      screen: state === "record" ? { kind: "workspace" } : { kind: "main" },
      overviewOpen: false,
      mainView: state === "projects" ? "projects" : (state === "work" || state === "focus") ? "work" : state === "history" ? "history" : state === "usage" ? "usage" : "backlog",
      overview: { ...OVERVIEW_ENTRY, panel: state === "home-record" ? { kind: "record", session: world.recordSession } : state === "backlog-detail" ? { kind: "item", workspaceId: "hide", taskKey: hideKey(163) } : null },
      quickAdd: state === "backlog-add" ? { returnFocus: null } : null,
      laneShown: state === "record",
    });
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
    root.classList.toggle(PALETTE_HIDE_CLASS, palette === "hide");
  }, [world, state, theme, palette]);
  return (
    <TooltipProvider>
      <div className="flex h-full flex-col bg-background text-foreground" data-gallery-scene="overview" data-overview-scene={state}>
        <TitleRow actions={actions} />
        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          {state === "record" ? <WorkspaceScreen actions={actions} /> : <OverviewPage actions={actions} />}
        </main>
        <QuickAdd actions={actions} />
        <Palette actions={actions} />
      </div>
    </TooltipProvider>
  );
}
