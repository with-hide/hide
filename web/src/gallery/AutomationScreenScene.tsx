// The automation screen (PRD automations B1-B8, B11, B16-B18, B25-B27, B30) in
// the production shell over invented data: the sidebar with its `자동화` row,
// and the screen in each state it carries. `table` is three automations, one
// running, one paused and one whose last run failed; `empty` has none;
// `unreadable` is the table under the line for a book set aside; `detail` is
// one automation's facts and runs; `form` makes a new one, its schedule read
// back by a stand-in core; `run` is a finished run whose terminal is closed, its summary with the agent's last answer and the results its agent opened (shown, not pressable: the run has no tab);
// `loading` is the section not yet sent. No daemon is used.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { configureAutomationCalls, receiveAutomationResult, type AutomationCall } from "../automationCall";
import { AutomationScreen } from "../AutomationScreen";
import { TooltipProvider } from "../components/ui/tooltip";
import { Sidebar } from "../sidebar";
import type { Automation, AutomationsSection } from "../snapshot";
import { useShellStore } from "../store";
import { TitleRow } from "../TitleRow";
import { AUTOMATION_ENTRY, useUiStore, type AutomationPlace } from "../ui";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";
import { sceneParams, type SceneParams } from "./SidebarScene";

const STATES = ["table", "empty", "unreadable", "detail", "form", "run", "loading"] as const;
type AutomationScreenState = (typeof STATES)[number];

type AutomationScreenParams = SceneParams & { state: AutomationScreenState };

export function automationScreenSceneParams(params: URLSearchParams): AutomationScreenParams {
  const state = params.get("state") ?? "table";
  if (!(STATES as readonly string[]).includes(state)) throw new Error(`Unknown automation screen state ${state}`);
  return { ...sceneParams(params), state: state as AutomationScreenState };
}

const HOUR = 3_600_000;
const ROOT = "/Users/me/hide/automations";

function automations(project: string, now: number): Automation[] {
  return [
    {
      id: "a-issues", name: "아침 이슈 정리", task: "새로 올라온 이슈를 읽고 우선순위와 담당 후보를 정리해 줘.",
      schedule: { kind: "weekdays", time: "09:00" }, reads_as: { kind: "weekdays", time: "09:00" }, next_run_at_unix_ms: now + 15 * HOUR,
      agent: { kind: "claude", model: "opus", effort: "high" }, folder: { path: `${ROOT}/아침-이슈-정리-a1`, default: true, worktree: false }, running: false, unseen: 1, recent_spend: { tokens: 16_440, micro_usd: 510_000, unpriced: true, runs: 2 },
      records: [
        { kind: "run", number: 4, trigger: "scheduled", due_unix_ms: now - 9 * HOUR, started_at_unix_ms: now - 9 * HOUR, ended_at_unix_ms: now - 9 * HOUR + 12 * 60_000, state: "succeeded", attempt: 1, opened: false, resumable: true, conversation_missing: false, seen: false, spend: { tokens: 12_340, micro_usd: 420_000, unpriced: false }, answer: "새 이슈 3건에 라벨을 붙였고, 먼저 올라온 이슈와 겹치는 1건은 닫았어요. 사람이 볼 건 없어요.", results: [{ key: "0a1b2c3d4e5f6071", kind: "file", label: "triage-report.md" }, { key: "1b2c3d4e5f607182", kind: "diff", label: "labels.diff" }, { key: "2c3d4e5f60718293", kind: "browser", label: "Issues - with-hide/hide" }] },
        { kind: "run", number: 3, trigger: "manual", started_at_unix_ms: now - 30 * HOUR, ended_at_unix_ms: now - 30 * HOUR + 8 * 60_000, state: "stopped", reason: "stopped_by_person", attempt: 1, opened: false, resumable: true, conversation_missing: false, seen: true, spend: { tokens: 4_100, micro_usd: 90_000, unpriced: true } },
        { kind: "skipped", at_unix_ms: now - 33 * HOUR, last_unix_ms: now - 29 * HOUR, count: 5, reason: { kind: "usage_limit", agent: "claude" } },
        { kind: "missed", first_unix_ms: now - 81 * HOUR, last_unix_ms: now - 57 * HOUR, count: 2, reason: "core_off" },
        { kind: "run", number: 2, trigger: "missed", due_unix_ms: now - 81 * HOUR, started_at_unix_ms: now - 56 * HOUR, ended_at_unix_ms: now - 56 * HOUR + 14 * 60_000, state: "succeeded", attempt: 1, opened: false, resumable: false, conversation_missing: true, seen: true },
      ],
    },
    {
      id: "a-deps", name: "의존성 점검", task: "lockfile의 오래된 의존성과 보안 공지를 확인하고 올릴 것만 PR로 묶어 줘.",
      schedule: { kind: "weekly", days: ["mon", "thu"], time: "10:00" }, reads_as: { kind: "weekly", days: ["mon", "thu"], time: "10:00" }, next_run_at_unix_ms: null,
      agent: { kind: "codex", model: "gpt-5", effort: "medium" }, folder: { path: project, default: false, worktree: true }, paused: { reason: "operator", at_unix_ms: now - 50 * HOUR }, running: false, unseen: 0,
      records: [
        { kind: "run", number: 1, trigger: "scheduled", due_unix_ms: now - 74 * HOUR, started_at_unix_ms: now - 74 * HOUR, ended_at_unix_ms: now - 74 * HOUR + 3 * 60_000, state: "failed", reason: "stopped_without_answer", attempt: 2, opened: false, resumable: true, conversation_missing: false, seen: true },
      ],
    },
    {
      id: "a-logs", name: "로그 정리", task: "어제 쌓인 진단 로그에서 같은 실패를 묶고 처음 보는 것만 알려 줘.",
      schedule: { kind: "every_minutes", minutes: 30 }, reads_as: { kind: "every_minutes", minutes: 30 }, next_run_at_unix_ms: now + 19 * 60_000,
      agent: { kind: "claude" }, folder: { path: `${ROOT}/로그-정리-a3`, default: true, worktree: false }, running: true, unseen: 0,
      records: [
        { kind: "run", number: 7, trigger: "scheduled", due_unix_ms: now - 11 * 60_000, started_at_unix_ms: now - 11 * 60_000, state: "running", attempt: 1, opened: false, resumable: false, conversation_missing: false, seen: true },
      ],
    },
  ];
}

/** A stand-in core: a schedule reads back as itself with its next run; an opening is taken and opens nothing, so a closed run stays closed. */
function standInCore(now: number) {
  configureAutomationCalls((event) => {
    if (event.kind !== "automation_call") return true;
    const { request_id, call } = event.payload as { request_id: string; call: AutomationCall };
    queueMicrotask(() => {
      if (call.call === "preview") receiveAutomationResult({ request_id, ok: true, value: { reads_as: call.schedule, next_run_at_unix_ms: now + 15 * HOUR, zone: null, own_folder: `${ROOT}/${call.name.replace(/\s+/g, "-")}${call.name ? "-" : ""}…` } });
      else if (call.call === "run_open") receiveAutomationResult({ request_id, ok: true, value: { id: call.id, run: call.run } });
      else receiveAutomationResult({ request_id, ok: false, code: "automations_unavailable" });
    });
    return true;
  });
}

export function AutomationScreenScene({ theme, scale, content, state }: AutomationScreenParams) {
  const fixture = useMemo(() => {
    const now = Date.now();
    const scene = sidebarScene(content, REFERENCE_FOLDS, now, "one");
    const hide = scene.rest.navigator!.workspaces!.find((project) => project.id === "hide")!;
    const items = state === "empty" ? [] : automations(hide.checkouts[0]!.path, now);
    const section: AutomationsSection | null = state === "loading" ? null : {
      items,
      unseen: items.reduce((sum, item) => sum + item.unseen, 0),
      last_choice: { kind: "claude" },
      own_folder_root: ROOT,
      ...(state === "unreadable" ? { unreadable: { file: `${ROOT}/automations.unreadable-1795.json`, at_unix_ms: now - HOUR } } : {}),
    };
    const place: AutomationPlace = {
      ...AUTOMATION_ENTRY,
      selected: state === "detail" || state === "run" ? "a-issues" : null,
      run: state === "run" ? 4 : null,
      form: state === "form" ? "new" : null,
    };
    return { scene, section, place, now };
  }, [content, state]);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    standInCore(fixture.now);
    useShellStore.setState({ rest: fixture.scene.rest, agents: fixture.scene.agents, automations: fixture.section, connection: "live" });
    useUiStore.setState({ screen: { kind: "automations" }, automations: fixture.place });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
  }, [fixture, theme, scale]);
  return (
    <TooltipProvider>
      <div className="flex h-full min-w-0 flex-col bg-background text-foreground" data-gallery-scene="automation-screen" data-automation-screen-scene={state}>
        <TitleRow actions={actions} />
        <div className="flex min-h-0 flex-1">
          <Sidebar actions={actions} />
          <main className="flex min-h-0 min-w-0 flex-1 flex-col">
            <AutomationScreen actions={actions} />
          </main>
        </div>
      </div>
    </TooltipProvider>
  );
}
