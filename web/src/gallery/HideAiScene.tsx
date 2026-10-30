// The production lane on its Hide AI tool over invented wire values, one
// state per screen the PRD names (hide-ai-suggestions B3, B10-B19, and
// memory-self-improve B5-B11 for the Hide global instruction), for
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

export const HIDE_AI_SCENE_STATES = ["new", "improvement", "applying", "applied", "settings", "settings_unread", "off", "first_run", "not_investigated", "memory", "empty", "memory_empty", "suggestions_off", "global", "global_unusable", "memory_offer", "asking", "ready"] as const;
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
const SIGNALS: SuggestionSignal[] = ["failed_tool", "agent_wait", "correction", "repeated_request", "memory_ignored", "label_missed", "improvement", "check_failed", "change_request"];

const row = (id: string, fields: Partial<SuggestionRow>): SuggestionRow => ({
  id, source: "notebook", group: "choose", state: "new", discomfort: null, cause: null, fix: null, outcome: "pr", basis: null,
  auto: false, hold: null, escalated: null, returned_after_skip: false, evidence: [], attempt: null, watch: null, ...fields,
});

const NEW_ROWS: SuggestionRow[] = [
  row("s12", {
    discomfort: "에이전트가 테스트를 돌릴 때마다 같은 곳에서 막혀요",
    cause: "cargo test 대신 scripts/verify-cargo.sh를 써야 한다는 말이 AGENTS.md에 없어요",
    fix: null,
    plan: { title: "Rust 검증은 verify-cargo.sh로만 돌리게 하기", path: "AGENTS.md", new_file: false, draft: "## Build Output Belongs To Its Worktree\n+ - Rust를 검증할 때는 cargo test 대신 scripts/verify-cargo.sh를 쓴다." },
    evidence: [
      { kind: "session", signal: "failed_tool", improvement: null, agent: "claude", session_id: "c1", pr_number: null, url: null, at_unix_ms: Date.now() - 40 * MINUTE, gone: false },
      { kind: "session", signal: "correction", improvement: null, agent: "codex", session_id: "c2", pr_number: null, url: null, at_unix_ms: Date.now() - 1440 * MINUTE, gone: false },
      { kind: "pr", signal: "change_request", improvement: null, agent: null, session_id: null, pr_number: 1688, url: "https://github.com/hide/hide/pull/1688", at_unix_ms: Date.now() - 3 * 1440 * MINUTE, gone: true },
    ],
  }),
  row("s14", { basis: { signal: "failed_tool", tag: "pnpm lint" }, cause: "lint가 무엇을 막는지 문서에 없어요", outcome: "issue" }),
  row("s16", { discomfort: "리뷰 전에 브랜치가 main보다 뒤처져 있어요", cause: "PR을 열기 전에 main을 합치라는 말이 없어요", plan: { title: "PR을 열기 전에 main을 합치게 하기", path: "CONTRIBUTING.md", new_file: false, draft: "+ PR을 열기 전에 main을 합친다." }, returned_after_skip: true }),
  row("s15", { discomfort: "PR마다 스크린샷이 빠졌다는 리뷰가 달려요", cause: "화면 PR의 스크린샷 규칙이 CONTRIBUTING.md에만 있어요", plan: { title: "화면 PR에 스크린샷을 붙이게 하기", path: "docs/screenshots.md", new_file: true, draft: "# 화면 PR의 스크린샷\n+ web/src를 바꾼 PR은 Summary에 바뀐 화면을 붙인다." }, hold: "github_needed" }),
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
    attempt: { phase: "working", started_at_unix_ms: Date.now() - 4 * MINUTE, pr_number: null, url: null, agent_pane: "hide-ai-s9", failure: null },
  }),
  row("s7", {
    group: "applying", state: "my_turn", discomfort: "리뷰 요청이 늦게 와요", cause: "PR을 열면 운영자에게 알리라는 말이 없어요", auto: true,
    attempt: { phase: "pr_open", started_at_unix_ms: Date.now() - 50 * MINUTE, pr_number: 1712, url: "https://github.com/hide/hide/pull/1712", agent_pane: null, failure: null },
  }),
  row("m72", {
    source: "memory", group: "applying", state: "my_turn", discomfort: "앱 교체는 운영자가 요청할 때만 한다", cause: "이 기억이 아직 문서에 없어요",
    attempt: { phase: "ci_failed", started_at_unix_ms: Date.now() - 3 * 60 * MINUTE, pr_number: 1709, url: "https://github.com/hide/hide/pull/1709", agent_pane: null, failure: null },
  }),
];

const APPLIED_ROWS: SuggestionRow[] = [
  row("s3", { group: "applied", state: "watching", discomfort: "에이전트가 앱을 마음대로 교체해요", attempt: { phase: "merged", started_at_unix_ms: Date.now() - 2 * 1440 * MINUTE, pr_number: 1690, url: "https://github.com/hide/hide/pull/1690", agent_pane: null, failure: null }, watch: { sessions_seen: 2, sessions_needed: 5 } }),
  row("s2", { group: "applied", state: "effective", discomfort: "커밋 메시지에 도구 이름이 들어가요", attempt: { phase: "merged", started_at_unix_ms: Date.now() - 6 * 1440 * MINUTE, pr_number: 1661, url: "https://github.com/hide/hide/pull/1661", agent_pane: null, failure: null } }),
];

/** A Memory card offered for the documents, opened to its text and evidence. */
const MEMORY_OFFER: SuggestionRow = row("m569", {
  source: "memory", discomfort: "premerge-check는 공용 잠금으로 한 번에 하나씩", outcome: "pr",
  memory: { body: "premerge-check를 여러 개 동시에 돌리면 부하가 200을 넘겨 20분 넘게 걸려요. lockf -k /tmp/hide-premerge.lock으로 감싸 한 번에 하나씩 돌려요.", human_sessions: 3, pulled_sessions: 2 },
});

/** The applying agent stopped on a question for the operator. */
const ASKING_ROW: SuggestionRow = { ...APPLYING_ROWS[0]!, plan: { title: "작업 트리를 나눠 쓰게 하기", path: "AGENTS.md", new_file: false, draft: "+ 브랜치마다 작업 트리를 따로 연다." } };

/** A branch Hide AI finished, opened to its files and the first one's diff. */
const READY_ROW: SuggestionRow = row("s12", {
  ...NEW_ROWS[0]!, group: "applying", state: "my_turn",
  attempt: {
    phase: "ready", started_at_unix_ms: Date.now() - 6 * MINUTE, pr_number: null, url: null, agent_pane: null, failure: null, purpose: "branch",
    branch: { name: "hide-suggestion-s12-verify", base: "main", checkout: "/projects/herdr-ide-s12", files: [{ path: "AGENTS.md", added: 1, removed: 0, binary: false }, { path: "docs/BUILD.md", added: 16, removed: 0, binary: false }], files_total: 2, added: 17, removed: 0 },
  },
});

export const HIDE_AI_SCENE_AGENTS = [{
  id: "hide-ai-s9", pane_id: "hide-ai-s9", agent_kind: "claude", identity_label: "작업 트리 나누기", mark: "turn", status_code: "question", group: "",
  state: { asking: true, working: false, line: { text: "AGENTS.md와 CONTRIBUTING.md 중 어디에 넣을까요?", mode: "request" } },
}];

export const HIDE_AI_SCENE_DIFFS = [{
  workspace_id: "herdr-ide", row: "s12", path: "AGENTS.md", reading: false, failed: false, notice: null,
  text: "diff --git a/AGENTS.md b/AGENTS.md\n--- a/AGENTS.md\n+++ b/AGENTS.md\n@@ -142,6 +142,7 @@ ## Build Output Belongs To Its Worktree\n - `scripts/verify-cargo.sh` and `scripts/verify-web.sh` are the only Rust and web verification entrypoints.\n+- Rust를 검증할 때는 cargo test 대신 scripts/verify-cargo.sh를 쓴다.\n - Every script that calls cargo sources `scripts/toolchain-env.sh`.\n",
}];

const GLOBAL_PATH = "~/.hide/state/global-instruction.md";

/** A personal request said again across sessions, opened with its words read back masked, one of them no longer readable (B6, B7). */
const LOCAL_NEW: SuggestionRow = row("s21", {
  discomfort: "답을 한국어로 해 달라고 계속 말해요",
  basis: { signal: "repeated_request", tag: "answer in korean" },
  outcome: "local",
  local: { line: "운영자가 한국어로 물으면 한국어로 답한다.", preview: "p1", present: false, applied_at_unix_ms: null, recurred: false, failure: null },
  evidence: [
    { kind: "session", signal: "repeated_request", improvement: null, agent: "claude", session_id: "r1", pr_number: null, url: null, at_unix_ms: Date.now() - 50 * MINUTE, gone: false, excerpt: "한국어로 답해 줘. 앞에서도 말했는데 영어로 왔어.", unreadable: false },
    { kind: "session", signal: "repeated_request", improvement: null, agent: "codex", session_id: "r2", pr_number: null, url: null, at_unix_ms: Date.now() - 1440 * MINUTE, gone: false, excerpt: "답은 한국어로 부탁해", unreadable: false },
    { kind: "session", signal: "repeated_request", improvement: null, agent: "claude", session_id: "r3", pr_number: null, url: null, at_unix_ms: Date.now() - 3 * 1440 * MINUTE, gone: false, excerpt: null, unreadable: true },
  ],
});

/** A line that went in and was said again in a session given it (B11). */
const LOCAL_RECURRED: SuggestionRow = row("s22", {
  discomfort: "em dash를 쓰지 말라고 또 말했어요",
  basis: { signal: "memory_ignored", tag: "m31" },
  outcome: "local",
  local: { line: "글에 em dash를 쓰지 않고 하이픈을 쓴다.", preview: "p2", present: false, applied_at_unix_ms: Date.now() - 2 * 1440 * MINUTE, recurred: true, failure: null },
});

const LOCAL_APPLIED: SuggestionRow = row("s20", {
  group: "applied", state: "applied", discomfort: "커밋 제목을 영어로 쓰라고 매번 말해요", outcome: "local",
  local: { line: "커밋 제목은 영어 명령문으로 쓴다.", preview: null, present: true, applied_at_unix_ms: Date.now() - 4 * 1440 * MINUTE, recurred: false, failure: null },
});

/** The file grew past its limit by hand: sessions are not given it, and a line cannot go in (B5, B10). */
const LOCAL_HELD: SuggestionRow = { ...LOCAL_NEW, evidence: [], local: { ...LOCAL_NEW.local!, preview: null } };

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
    case "empty":
    case "memory_empty": return { ...base, status: "none_today", rows: [], counts: { suggestions: 0, rail: 0 } };
    case "global": return { ...base, global: { path: GLOBAL_PATH, unusable: null }, rows: [LOCAL_NEW, LOCAL_RECURRED, ...NEW_ROWS.slice(0, 1), LOCAL_APPLIED, ...APPLIED_ROWS], counts: { suggestions: 3, rail: 4 } };
    case "global_unusable": return { ...base, global: { path: GLOBAL_PATH, unusable: "too_large" }, rows: [LOCAL_HELD, ...NEW_ROWS.slice(1, 2)], counts: { suggestions: 2, rail: 3 } };
    case "memory_offer": return { ...base, rows: [MEMORY_OFFER, ...NEW_ROWS.slice(0, 2)], counts: { suggestions: 3, rail: 3 } };
    case "asking": return { ...base, rows: [ASKING_ROW, ...APPLYING_ROWS.slice(1), ...APPLIED_ROWS], counts: { suggestions: 2, rail: 3 } };
    case "ready": return { ...base, rows: [READY_ROW, ...APPLYING_ROWS.slice(1), ...APPLIED_ROWS], counts: { suggestions: 3, rail: 4 } };
    case "suggestions_off": return { ...base, status: "off", settings: { ...base.settings, suggestions_enabled: false }, counts: { suggestions: 0, rail: 0 } };
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
    const memory = { ...memorySceneProject(state === "off" || state === "settings_unread" ? "off" : state === "memory_empty" ? "empty" : "correction"), workspace_id: "herdr-ide" };
    const shown = state === "ready" ? { ...rest, pr_work: { link: null, feedback: null, branch_diffs: HIDE_AI_SCENE_DIFFS } } as SnapshotRest : rest;
    useShellStore.setState({ rest: shown, agents: (state === "asking" ? HIDE_AI_SCENE_AGENTS : []) as never, memory: { project: memory }, suggestions: { project: hideAiSceneProject(state) }, connection: "gone" });
    useUiStore.setState({ hideAiTab: state === "memory" || state === "memory_empty" ? "memory" : "suggestions", openSuggestion: state === "new" ? "s12" : state === "improvement" ? "s18" : state === "global" ? "s21" : state === "memory_offer" ? "m569" : state === "ready" ? "s12" : null });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [rest, state, theme]);
  // The fold and the sheet are the components' own; the scene presses them as the operator would, once the tool drew them.
  const drawn = useShellStore((store) => store.suggestions?.project?.workspace_id === "herdr-ide");
  useEffect(() => {
    const press = state === "applied" || state === "global" ? "[data-suggestions-fold]" : state === "settings" || state === "settings_unread" ? "[data-hide-ai-settings-open]" : null;
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
