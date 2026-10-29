// The production lane on its Hide AI tool's Memory tab over invented wire values,
// one state per document, for comparison with the v7 design exports (PRD
// project-memory-v2 D-17, B19-B32). Runtime policy is tested in herdr-core.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { TooltipProvider } from "../components/ui/tooltip";
import type { Checkout, MemoryRow, ProjectMemory, SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { Lane } from "../Tools";
import { useUiStore } from "../ui";
import type { LaneFrame, LaneSizes, WorkspaceView } from "../workspace";
import { hideAiSceneProject } from "./HideAiScene";

export const MEMORY_SCENE_STATES = ["workspace", "correction", "off", "ai_not_ready", "analyzing", "empty", "unreadable", "forgotten", "verifying", "reconsent", "checking", "check_budget_exhausted", "evicted", "full", "upkeep"] as const;
export type MemorySceneState = (typeof MEMORY_SCENE_STATES)[number];
export type MemorySceneLanguage = "en" | "ko" | "zh-CN" | "ja";
export type MemorySceneParams = { theme: "light" | "dark"; state: MemorySceneState; width: number; language: MemorySceneLanguage };

export function memorySceneParams(params: URLSearchParams): MemorySceneParams {
  const state = params.get("state") ?? "workspace";
  if (!MEMORY_SCENE_STATES.includes(state as MemorySceneState)) throw new Error(`Unknown memory scene state ${state}`);
  const width = Number(params.get("width") ?? "360");
  if (!(width > 0)) throw new Error(`Scene width must be a positive number, got ${params.get("width")}`);
  const language = params.get("lang") ?? "ko";
  if (!["en", "ko", "zh-CN", "ja"].includes(language)) throw new Error(`Unknown memory scene language ${language}`);
  return { theme: params.get("theme") === "light" ? "light" : "dark", state: state as MemorySceneState, width, language: language as MemorySceneLanguage };
}

const MINUTE = 60_000;
const row = (number: number, title: string, kind: MemoryRow["kind"], given: number, pulled: number, human: number, ago: number, promotion: MemoryRow["promotion"] = null): MemoryRow => ({
  number, title, kind, provided_session_count: given, pull_count: pulled, human_source_count: human, updated_at_unix_ms: Date.now() - ago, verifying: false, promotion,
});
const ROWS: MemoryRow[] = [
  row(17, "스크린샷이 큰 세션을 읽다 막히면 종 알림이 멈추므로, 먼저 tool_result 크기를 확인한다", "lesson", 0, 0, 1, 60 * MINUTE),
  row(7, "앱 교체는 운영자가 요청할 때만 한다", "rule", 6, 4, 2, 5 * 1440 * MINUTE, { state: "running", agent: { name: "memory-promote-m7", pane_id: "w1:p9" }, pr_number: null, target_path: null, github_failure: null }),
  row(9, "라이선스는 FCL로 바꾸고 저장소는 공개로 둔다", "decision", 3, 2, 2, 3 * 1440 * MINUTE, { state: "pr_open", agent: null, pr_number: 924, target_path: null, github_failure: null }),
  row(10, "릴리스 노트는 운영자가 확인한 뒤 올린다", "rule", 2, 1, 1, 2 * 1440 * MINUTE, { state: "unconfirmed", agent: null, pr_number: null, target_path: null, github_failure: "not_logged_in" }),
  row(8, "하위 작업은 hide agent spawn --parent here로 띄운다", "rule", 7, 5, 1, 6 * 1440 * MINUTE, { state: "merged", agent: null, pr_number: 921, target_path: "AGENTS.md", github_failure: null }),
  row(12, "nextest 0.9.145 미만은 macOS에서 가짜 LEAK를 낸다", "lesson", 2, 1, 1, 12 * MINUTE),
  row(3, "Rust 검증은 scripts/verify-cargo.sh로만 돌린다", "rule", 9, 12, 3, 1440 * MINUTE),
  row(5, "커밋 메시지에 도구 이름을 넣지 않는다", "rule", 6, 4, 2, 4 * 1440 * MINUTE),
  ...Array.from({ length: 17 }, (_, index) => row(20 + index, `예시 카드 ${index + 1}`, "fact", 1, 0, 0, (7 + index) * 1440 * MINUTE)),
];
const EXPANDED: ProjectMemory["expanded"] = {
  number: 12,
  body: "0.9.145 미만 nextest는 macOS에서 가짜 LEAK를 내므로, 그 버전 이상을 쓰고 LEAK는 먼저 버전을 의심한다.",
  cited_paths: [".config/nextest.toml"],
  sources: [
    { provider_label: "Claude Code", at_unix_ms: Date.now() - 1440 * MINUTE, human: true },
    { provider_label: "Codex", at_unix_ms: Date.now() - 3 * 1440 * MINUTE, human: false },
  ],
  previous: { body: "0.9.132 미만 nextest는 macOS에서 가짜 LEAK를 낸다.", changed_at_unix_ms: Date.now() - 12 * MINUTE },
};

export function memorySceneProject(state: MemorySceneState): ProjectMemory {
  const base: ProjectMemory = {
    workspace_id: "herdr-ide", unavailable: null, loading: false, enabled: true, capacity_reached: false, evicted_cards: 0, evicted_through_unix_ms: null,
    rows: ROWS, questions: [], analysis: "idle", consent_required: false, usable_cards: 2, checking_card: null, check_resume_at_unix_ms: null, forgotten: null, pending_action: null, expanded: null, search: null,
  };
  switch (state) {
    case "workspace":
      return { ...base, expanded: EXPANDED };
    case "correction": return { ...base, rows: [{ ...ROWS[6]!, title: "Use the repository verification script", body_preview: "Use the verification script so local checks and CI use the same toolchain." }], questions: [{ kind: "docs_disagree", number: ROWS[6]!.number, title: "Use the repository verification script", document: { authority: "card_newer", document: "docs/TESTING.md", document_at_unix_ms: Date.now(), operator_at_unix_ms: Date.now() - 1440 * MINUTE, operator_line: "검증은 저장소 스크립트로 해. 로컬이랑 CI가 같아야 해", document_sentence: "Run cargo test and pnpm test directly before a merge.", replacement: "Run the repository verification script before a merge." } }] };
    case "off": return { ...base, enabled: false, usable_cards: 0 };
    case "ai_not_ready": return { ...base, analysis: "ai_not_ready" };
    case "analyzing": return { ...base, analysis: "analyzing" };
    case "empty": return { ...base, rows: [], usable_cards: 0 };
    case "unreadable": return { ...base, unavailable: { reason: "unreadable" }, rows: [] };
    case "reconsent": return { ...base, enabled: false, usable_cards: 0, consent_required: true };
    case "checking": return { ...base, usable_cards: 0, analysis: "checking", checking_card: 17, rows: [{ ...ROWS[0]!, verifying: true }, { ...ROWS[5]!, verifying: true }] };
    case "check_budget_exhausted": return { ...base, usable_cards: 0, analysis: "check_budget_exhausted", check_resume_at_unix_ms: (Math.floor(Date.now() / 86_400_000) + 1) * 86_400_000, rows: [{ ...ROWS[0]!, verifying: true }, { ...ROWS[5]!, verifying: true }] };
    case "verifying": return { ...base, rows: [{ ...ROWS[0]!, verifying: true }, ROWS[5]!, { ...ROWS[6]!, verifying: true }] };
    case "forgotten": return { ...base, rows: ROWS.filter((card) => card.number !== 9), forgotten: 9 };
    case "evicted": return { ...base, evicted_cards: 3, evicted_through_unix_ms: Date.now() - 5 * MINUTE };
    case "full": return { ...base, capacity_reached: true, evicted_cards: 3, evicted_through_unix_ms: Date.now() - 5 * MINUTE };
    // What Memory keeps by itself (PRD memory-self-improve B21, B22, B27, B34):
    // an unused card's mark, a card's change history and the archive.
    case "upkeep": return {
      ...base,
      rows: [ROWS[5]!, { ...ROWS[7]!, stale: true }, ROWS[6]!],
      expanded: {
        ...EXPANDED!,
        previous: null,
        changes: [
          { id: 52, kind: "recheck", by_agent: true, letters: 1, unreadable: 1, at_unix_ms: Date.now() - 30 * MINUTE, undone: false, undo_blocked: "recheck_finished" },
          { id: 47, kind: "alias", aliases: ["누수", "LEAK"], at_unix_ms: Date.now() - 2 * 1440 * MINUTE, undone: false, undo_blocked: null },
          { id: 31, kind: "merge", cards: [14, 15], at_unix_ms: Date.now() - 6 * 1440 * MINUTE, undone: false, undo_blocked: null },
        ],
      },
      archived: [
        { number: 2, title: "배포 전에는 staging 슬랙 채널에 알린다", archived_at_unix_ms: Date.now() - 3 * 1440 * MINUTE },
        { number: 4, title: "README 스크린샷은 다크 테마로 찍는다", archived_at_unix_ms: Date.now() - 9 * 1440 * MINUTE },
      ],
    };
  }
}

/** The lane pinned on the Hide AI tool (lane id `memory`), the scene's width wide. */
const VIEW: WorkspaceView = { device_id: "local", path: "/projects/herdr-ide", open: true, documents: false, tool: "memory", pinned: true, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view" };
const FRAME: LaneFrame = { mode: "docked", agents: 0, width: 0, view: null, tool: { field: "panel_width", width: 0, min: 360 }, stacked: false, narrowPin: false };
const SIZES: LaneSizes = { agentMin: 480, viewMin: 360, viewIdeal: 520, lanePage: 900, explorerMin: 180, explorerIdeal: 240, panelMin: 360, panelIdeal: 400, divider: 8 };

export function MemoryScene({ theme, state, width, language }: MemorySceneParams) {
  const actions = useMemo(() => createActions((event) => {
    if (event.kind === "memory_expand") {
      const project = useShellStore.getState().memory?.project;
      if (!project) return false;
      const card = project.rows.find((row) => row.number === event.payload.number);
      useShellStore.setState({ memory: { project: { ...project, expanded: card ? { number: card.number, body: card.body_preview ?? card.title, cited_paths: [], sources: [], previous: null } : null } } });
    }
    return true;
  }), []);
  const rest = useMemo(() => ({
    navigator: {
      focused_device_id: "local", focused_checkout_id: "main", focused_workspace_id: "herdr-ide",
      devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }],
      workspaces: [{ id: "herdr-ide", label: "herdr-ide", device_id: "local", path: "/projects/herdr-ide", checkouts: [{ id: "main", workspace_id: "herdr-ide", path: "/projects/herdr-ide", label: "main", tabs: [], active_tab_id: null, strip: [] }] }],
    },
    status: { background_ai: { enabled: state !== "ai_not_ready", provider: "claude", chosen: true, unavailable_reason: null, providers: [{ id: "claude", label: "Claude Code", agent: "claude-code", state: "ready", selectable: true }] } },
    ui_state: { interface_language: language },
  }) as unknown as SnapshotRest, [state, language]);
  useLayoutEffect(() => {
    useShellStore.setState({ rest, memory: { project: memorySceneProject(state) }, suggestions: { project: { ...hideAiSceneProject(state === "off" ? "off" : "memory"), counts: { suggestions: 5, rail: 5 } } }, connection: "gone" });
    useUiStore.setState({ hideAiTab: "memory" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [rest, state, theme]);
  return (
    <TooltipProvider>
      <InterfaceLanguageBoundary />
      <div className="flex h-screen bg-background text-foreground" style={{ width }} data-gallery-scene="memory" data-memory-scene={state}>
        <Lane view={{ ...VIEW, panel_width: width }} frame={{ ...FRAME, width, tool: { field: "panel_width", width, min: 360 } }} documents={{ ...FRAME, width, tool: null, view: { field: "document_width", width, min: 360 } }} sizes={SIZES} checkout={rest.navigator!.workspaces![0]!.checkouts[0] as Checkout} actions={actions} />
      </div>
    </TooltipProvider>
  );
}
