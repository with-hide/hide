// The production lane on its Memory tool over invented wire values,
// one state per document, for comparison with the v7 design exports (PRD
// project-memory-v2 D-17, B19-B32). Runtime policy is tested in herdr-core.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { InterfaceLanguageBoundary } from "../i18n/client";
import { TooltipProvider } from "../components/ui/tooltip";
import type { Checkout, MemoryRow, ProjectMemory, SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { Lane } from "../Tools";
import type { LaneFrame, LaneSizes, WorkspaceView } from "../workspace";

export const MEMORY_SCENE_STATES = ["workspace", "correction", "off", "ai_not_ready", "analyzing", "empty", "unreadable", "promote", "writing", "writing_batch", "failed", "start_failed", "not_processed", "building", "unbuilt", "forgotten", "verifying", "reconsent", "checking", "check_budget_exhausted", "evicted", "full"] as const;
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
/** A decision the main app does not have yet, which another branch builds. */
const UNBUILT = row(14, "백로그는 프로젝트 이슈 출처 하나만 따른다", "decision", 2, 1, 1, 3 * 60 * MINUTE);
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
      return { ...base, expanded: EXPANDED, questions: [
        { kind: "promote", number: 3, title: ROWS[5]!.title, human_session_count: 3, pulled_session_count: 9 },
        { kind: "promote", number: 5, title: ROWS[6]!.title, human_session_count: 2, pulled_session_count: 6 },
      ] };
    case "correction": return { ...base, rows: [{ ...ROWS[6]!, title: "Use the repository verification script", body_preview: "Use the verification script so local checks and CI use the same toolchain." }], questions: [{ kind: "docs_disagree", number: ROWS[6]!.number, title: "Use the repository verification script", document: { authority: "card_newer", document: "docs/TESTING.md", document_at_unix_ms: Date.now(), operator_at_unix_ms: Date.now() - 1440 * MINUTE } }] };
    case "off": return { ...base, enabled: false, usable_cards: 0 };
    case "ai_not_ready": return { ...base, analysis: "ai_not_ready" };
    case "analyzing": return { ...base, analysis: "analyzing" };
    case "empty": return { ...base, rows: [], usable_cards: 0 };
    case "unreadable": return { ...base, unavailable: { reason: "unreadable" }, rows: [] };
    case "promote": return { ...base, rows: ROWS.slice(1, 4).map((card, index) => ({ ...card, body_preview: [
      "설치된 앱을 교체하기 전 운영자의 요청과 현재 버전을 확인하고 기존 앱을 백업한다.",
      "저장소는 공개로 유지하며 라이선스 변경 내용은 저장소의 라이선스 파일에 반영한다.",
      "릴리스 노트는 게시 전 운영자에게 보여주고 확인받은 내용으로 올린다.",
    ][index] })) };
    case "writing":
    case "writing_batch": {
      const promotion = { ...ROWS[1]!.promotion!, started_at_unix_ms: Date.now() - 4 * MINUTE };
      const writing = { ...ROWS[1]!, promotion, body_preview: "설치된 앱은 운영자가 요청했을 때만 교체한다. 교체 전 버전을 확인하고 기존 앱을 백업한다." };
      return { ...base, rows: state === "writing" ? [writing] : [writing, { ...ROWS[6]!, promotion, body_preview: "Rust 검증은 저장소가 제공하는 검증 스크립트로 실행한다." }] };
    }
    case "failed": return { ...base, rows: [ROWS[3]!], questions: [{ kind: "failed", number: 8, title: ROWS[3]!.title, start_failure: null }] };
    case "start_failed": return { ...base, rows: [ROWS[3]!], questions: [{ kind: "failed", number: 8, title: ROWS[3]!.title, start_failure: { code: "shell_starting", detail: "[oh-my-zsh] Would you like to update? [Y/n]" } }] };
    case "not_processed": return { ...base, rows: [UNBUILT, ROWS[6]!], expanded: { number: 14, body: UNBUILT.title, cited_paths: [], sources: [], previous: null }, questions: [{ kind: "failed", number: 14, title: UNBUILT.title, start_failure: { code: "card_not_processed", detail: "백로그와 메모가 아직 main 앱에 없어 문서를 고치지 않았습니다." } }] };
    case "building": return { ...base, rows: [{ ...UNBUILT, promotion: { state: "building", agent: { name: "impl-overview-tasks", pane_id: "w2:p1" }, pr_number: null, target_path: "docs/UI_BEHAVIOR.md", github_failure: null } }, ROWS[6]!, ROWS[7]!] };
    case "unbuilt": return { ...base, rows: [UNBUILT, ROWS[6]!, ROWS[7]!], questions: [{ kind: "unbuilt", number: 14, title: UNBUILT.title, reason: "백로그 출처를 고르는 화면이 아직 없고, 만드는 브랜치도 없습니다." }] };
    case "reconsent": return { ...base, enabled: false, usable_cards: 0, consent_required: true };
    case "checking": return { ...base, usable_cards: 0, analysis: "checking", checking_card: 17, rows: [{ ...ROWS[0]!, verifying: true }, { ...ROWS[5]!, verifying: true }] };
    case "check_budget_exhausted": return { ...base, usable_cards: 0, analysis: "check_budget_exhausted", check_resume_at_unix_ms: (Math.floor(Date.now() / 86_400_000) + 1) * 86_400_000, rows: [{ ...ROWS[0]!, verifying: true }, { ...ROWS[5]!, verifying: true }] };
    case "verifying": return { ...base, rows: [{ ...ROWS[0]!, verifying: true }, ROWS[5]!, { ...ROWS[6]!, verifying: true }] };
    case "forgotten": return { ...base, rows: ROWS.filter((card) => card.number !== 9), forgotten: 9 };
    case "evicted": return { ...base, evicted_cards: 3, evicted_through_unix_ms: Date.now() - 5 * MINUTE };
    case "full": return { ...base, capacity_reached: true, evicted_cards: 3, evicted_through_unix_ms: Date.now() - 5 * MINUTE };
  }
}

/** The lane pinned on its Memory tool, the scene's width wide. */
const VIEW: WorkspaceView = { device_id: "local", path: "/projects/herdr-ide", open: true, documents: false, tool: "memory", prs: { only_checkout: false }, pinned: true, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view" };
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
    useShellStore.setState({ rest, memory: { project: memorySceneProject(state) }, connection: "gone" });
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
