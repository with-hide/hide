// The production lane on its Hide AI tool over invented wire values, one
// state per screen the PRD names (hide-ai-suggestions B3, B10-B19), for
// review captures. Which rows exist, their groups and counts are the core's
// and are tested in herdr-core.
import { useEffect, useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { TooltipProvider } from "../components/ui/tooltip";
import type { Checkout, ProjectSuggestions, SnapshotRest, SuggestionRow, SuggestionSignal } from "../snapshot";
import { useShellStore } from "../store";
import { Lane } from "../Tools";
import { useUiStore } from "../ui";
import type { LaneFrame, LaneSizes, WorkspaceView } from "../workspace";
import { memorySceneProject } from "./MemoryScene";

export const HIDE_AI_SCENE_STATES = ["new", "improvement", "applying", "applied", "settings", "settings_unread", "off", "first_run", "not_investigated", "memory"] as const;
export type HideAiSceneState = (typeof HIDE_AI_SCENE_STATES)[number];
export type HideAiSceneLanguage = "en" | "ko" | "zh-CN" | "ja";
export type HideAiSceneParams = { theme: "light" | "dark"; state: HideAiSceneState; width: number; language: HideAiSceneLanguage };

export function hideAiSceneParams(params: URLSearchParams): HideAiSceneParams {
  const state = params.get("state") ?? "new";
  if (!HIDE_AI_SCENE_STATES.includes(state as HideAiSceneState)) throw new Error(`Unknown Hide AI scene state ${state}`);
  const width = Number(params.get("width") ?? "400");
  if (!(width > 0)) throw new Error(`Scene width must be a positive number, got ${params.get("width")}`);
  const language = params.get("lang") ?? "ko";
  if (!["en", "ko", "zh-CN", "ja"].includes(language)) throw new Error(`Unknown Hide AI scene language ${language}`);
  return { theme: params.get("theme") === "light" ? "light" : "dark", state: state as HideAiSceneState, width, language: language as HideAiSceneLanguage };
}

const MINUTE = 60_000;
const SIGNALS: SuggestionSignal[] = ["failed_tool", "agent_wait", "correction", "label_missed", "improvement", "check_failed", "change_request"];

const row = (id: string, fields: Partial<SuggestionRow>): SuggestionRow => ({
  id, source: "notebook", group: "choose", state: "new", discomfort: null, cause: null, fix: null, outcome: "pr", basis: null,
  auto: false, hold: null, escalated: null, returned_after_skip: false, evidence: [], attempt: null, watch: null, ...fields,
});

const NEW_ROWS: SuggestionRow[] = [
  row("s12", {
    discomfort: "에이전트가 테스트를 돌릴 때마다 같은 곳에서 막혀요",
    cause: "cargo test 대신 scripts/verify-cargo.sh를 써야 한다는 말이 AGENTS.md에 없어요",
    fix: "AGENTS.md의 Build Output 절에 Rust 검증은 scripts/verify-cargo.sh로만 돌린다는 줄을 넣어요.",
    evidence: [
      { kind: "session", signal: "failed_tool", improvement: null, agent: "claude", session_id: "c1", pr_number: null, url: null, at_unix_ms: Date.now() - 40 * MINUTE, gone: false },
      { kind: "session", signal: "correction", improvement: null, agent: "codex", session_id: "c2", pr_number: null, url: null, at_unix_ms: Date.now() - 1440 * MINUTE, gone: false },
      { kind: "pr", signal: "change_request", improvement: null, agent: null, session_id: null, pr_number: 1688, url: "https://github.com/hide/hide/pull/1688", at_unix_ms: Date.now() - 3 * 1440 * MINUTE, gone: true },
    ],
  }),
  row("s14", { basis: { signal: "failed_tool", tag: "pnpm lint" }, cause: "lint가 무엇을 막는지 문서에 없어요", outcome: "issue" }),
  row("s16", { discomfort: "리뷰 전에 브랜치가 main보다 뒤처져 있어요", cause: "PR을 열기 전에 main을 합치라는 말이 없어요", fix: "CONTRIBUTING.md에 PR을 열기 전 main을 합치는 줄을 넣어요.", returned_after_skip: true }),
  row("s15", { discomfort: "PR마다 스크린샷이 빠졌다는 리뷰가 달려요", cause: "화면 PR의 스크린샷 규칙이 CONTRIBUTING.md에만 있어요", hold: "github_needed" }),
];

/** A line that rose from improvements alone, each session's evidence naming how it lost time (B25). */
const IMPROVEMENT_ROW: SuggestionRow = row("s18", {
  discomfort: "포트 규칙을 몰라서 매번 찾아요",
  cause: "dev 서버는 5173, e2e는 5174라는 규칙이 어디에도 없어요",
  fix: "AGENTS.md의 Repository Layout 아래에 한 줄을 넣어요: \"web dev 서버는 5173, Playwright e2e는 5174를 쓴다. 바꾸려면 web/playwright.config.ts의 PORT 하나만 고친다.\"",
  evidence: [
    { kind: "session", signal: "improvement", improvement: "no_doc", agent: "claude", session_id: "i1", pr_number: null, url: null, at_unix_ms: Date.now() - 2 * 1440 * MINUTE, gone: false },
    { kind: "session", signal: "improvement", improvement: "searched_again", agent: "codex", session_id: "i2", pr_number: null, url: null, at_unix_ms: Date.now() - 1440 * MINUTE, gone: false },
    { kind: "session", signal: "improvement", improvement: "wrong_guess", agent: "claude", session_id: "i3", pr_number: null, url: null, at_unix_ms: Date.now() - 90 * MINUTE, gone: false },
  ],
});

const APPLYING_ROWS: SuggestionRow[] = [
  row("s9", {
    group: "applying", state: "hide_ai_working", discomfort: "에이전트가 같은 파일을 두 번 고쳐요", cause: "작업 트리를 나누라는 말이 없어요",
    attempt: { phase: "working", started_at_unix_ms: Date.now() - 4 * MINUTE, pr_number: null, url: null, agent_id: "hide-ai-s9", failure: null },
  }),
  row("s7", {
    group: "applying", state: "my_turn", discomfort: "리뷰 요청이 늦게 와요", cause: "PR을 열면 운영자에게 알리라는 말이 없어요", auto: true,
    attempt: { phase: "pr_open", started_at_unix_ms: Date.now() - 50 * MINUTE, pr_number: 1712, url: "https://github.com/hide/hide/pull/1712", agent_id: null, failure: null },
  }),
  row("m72", {
    source: "memory", group: "applying", state: "my_turn", discomfort: "앱 교체는 운영자가 요청할 때만 한다", cause: "이 기억이 아직 문서에 없어요",
    attempt: { phase: "ci_failed", started_at_unix_ms: Date.now() - 3 * 60 * MINUTE, pr_number: 1709, url: "https://github.com/hide/hide/pull/1709", agent_id: null, failure: null },
  }),
];

const APPLIED_ROWS: SuggestionRow[] = [
  row("s3", { group: "applied", state: "watching", discomfort: "에이전트가 앱을 마음대로 교체해요", attempt: { phase: "merged", started_at_unix_ms: Date.now() - 2 * 1440 * MINUTE, pr_number: 1690, url: "https://github.com/hide/hide/pull/1690", agent_id: null, failure: null }, watch: { sessions_seen: 2, sessions_needed: 5 } }),
  row("s2", { group: "applied", state: "effective", discomfort: "커밋 메시지에 도구 이름이 들어가요", attempt: { phase: "merged", started_at_unix_ms: Date.now() - 6 * 1440 * MINUTE, pr_number: 1661, url: "https://github.com/hide/hide/pull/1661", agent_id: null, failure: null } }),
];

/** The Project's Hide AI as the core would send it in each state, for this scene and the lane scenes that show the tool. */
export function hideAiSceneProject(state: HideAiSceneState, workspaceId = "herdr-ide"): ProjectSuggestions {
  const base: ProjectSuggestions = {
    workspace_id: workspaceId, loading: false, unavailable: null, enabled: true, intro_seen: true, status: "idle", ai: "ready", github: "connected",
    settings: {
      suggestions_enabled: true, quantity: "normal", apply_mode: "confirm",
      signals: SIGNALS.map((signal) => ({ signal, enabled: true, unavailable: null })),
      dismissed: [
        { id: "s4", discomfort: "탭 이름이 너무 길어요", basis: null, may_return: true },
        { id: "s5", discomfort: "알림 소리가 커요", basis: null, may_return: false },
      ],
    },
    rows: [...NEW_ROWS, ...APPLYING_ROWS, ...APPLIED_ROWS],
    counts: { suggestions: 6, rail: 7 },
  };
  switch (state) {
    case "new":
    case "settings":
    case "memory": return base;
    case "improvement": return { ...base, rows: [IMPROVEMENT_ROW, ...NEW_ROWS, ...APPLYING_ROWS, ...APPLIED_ROWS], counts: { suggestions: 7, rail: 8 } };
    // Not on GitHub, with Memory and the turn-end analysis off: GitHub reads nothing, the session records read all but corrections and improvements (D-52, D-57).
    case "settings_unread": return {
      ...base,
      settings: {
        ...base.settings,
        signals: base.settings.signals.map((row) => (
          row.signal === "check_failed" || row.signal === "change_request" ? { ...row, enabled: false, unavailable: "no_github" as const }
            : row.signal === "correction" ? { ...row, enabled: false, unavailable: "memory_off" as const }
            : row.signal === "improvement" ? { ...row, enabled: false, unavailable: "analysis_off" as const } : row
        )),
      },
    };
    case "applying": return { ...base, rows: [...APPLYING_ROWS, ...APPLIED_ROWS], counts: { suggestions: 2, rail: 3 } };
    case "applied": return { ...base, status: "none_today", rows: APPLIED_ROWS, counts: { suggestions: 0, rail: 1 } };
    case "off": return { ...base, enabled: false, counts: { suggestions: 0, rail: 0 } };
    case "not_investigated": return { ...base, status: "not_investigated", rows: [...APPLYING_ROWS, ...APPLIED_ROWS], counts: { suggestions: 2, rail: 3 } };
    case "first_run": return { ...base, intro_seen: false, status: "first_run_pending", rows: [], counts: { suggestions: 0, rail: 1 } };
  }
}

/** The lane pinned on the Hide AI tool (lane id `memory`), the scene's width wide. */
const VIEW: WorkspaceView = { device_id: "local", path: "/projects/herdr-ide", open: true, documents: false, tool: "memory", pinned: true, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view" };
const FRAME: LaneFrame = { mode: "docked", agents: 0, width: 0, view: null, tool: { field: "panel_width", width: 0, min: 360 }, stacked: false, narrowPin: false };
const SIZES: LaneSizes = { agentMin: 480, viewMin: 360, viewIdeal: 520, lanePage: 900, explorerMin: 180, explorerIdeal: 240, panelMin: 360, panelIdeal: 400, divider: 8 };

export function HideAiScene({ theme, state, width, language }: HideAiSceneParams) {
  const actions = useMemo(() => createActions(() => true), []);
  const rest = useMemo(() => ({
    navigator: {
      focused_device_id: "local", focused_checkout_id: "main", focused_workspace_id: "herdr-ide",
      devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }],
      workspaces: [{ id: "herdr-ide", label: "herdr-ide", device_id: "local", path: "/projects/herdr-ide", checkouts: [{ id: "main", workspace_id: "herdr-ide", path: "/projects/herdr-ide", label: "main", tabs: [], active_tab_id: null, strip: [] }] }],
    },
    status: { background_ai: { enabled: true, provider: "claude", chosen: true, unavailable_reason: null, providers: [{ id: "claude", label: "Claude Code", agent: "claude-code", state: "ready", selectable: true }] } },
    ui_state: { interface_language: language },
  }) as unknown as SnapshotRest, [language]);
  useLayoutEffect(() => {
    const memory = { ...memorySceneProject(state === "off" || state === "settings_unread" ? "off" : "correction"), workspace_id: "herdr-ide" };
    useShellStore.setState({ rest, memory: { project: memory }, suggestions: { project: hideAiSceneProject(state) }, connection: "gone" });
    useUiStore.setState({ hideAiTab: state === "memory" ? "memory" : "suggestions", openSuggestion: state === "new" ? "s12" : state === "improvement" ? "s18" : null });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [rest, state, theme]);
  // The fold and the sheet are the components' own; the scene presses them as the operator would, once the tool drew them.
  const drawn = useShellStore((store) => store.suggestions?.project?.workspace_id === "herdr-ide");
  useEffect(() => {
    const press = state === "applied" ? "[data-suggestions-fold]" : state === "settings" || state === "settings_unread" ? "[data-hide-ai-settings-open]" : null;
    if (!drawn || !press) return;
    document.querySelector<HTMLElement>(press)?.click();
    if (state !== "settings_unread") return;
    // The skipped lines unfold inside the sheet once it has opened.
    const frame = requestAnimationFrame(() => document.querySelector<HTMLElement>("[data-hide-ai-dismissed] > button")?.click());
    return () => cancelAnimationFrame(frame);
  }, [drawn, state]);
  return (
    <TooltipProvider>
      <InterfaceLanguageBoundary />
      <div className="flex h-screen bg-background text-foreground" style={{ width }} data-gallery-scene="hide-ai" data-hide-ai-scene={state}>
        <Lane view={{ ...VIEW, panel_width: width }} frame={{ ...FRAME, width, tool: { field: "panel_width", width, min: 360 } }} documents={{ ...FRAME, width, tool: null, view: { field: "document_width", width, min: 360 } }} sizes={SIZES} checkout={rest.navigator!.workspaces![0]!.checkouts[0] as Checkout} actions={actions} />
      </div>
    </TooltipProvider>
  );
}
