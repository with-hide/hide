// The device Overview (PRD overview-device-tasks; PRD sidebar-task-lens D-20)
// in the production screens over invented device data
// (`overviewSceneData.ts`): the backlog with ⌘⇧M open, Projects after the
// disk sweep, a session's record beside the tabs at Home, and an agent pane
// whose head showed its record in the lane's 보기 slot. No daemon runs; the
// isolated daemon tests own runtime policy.
import { useLayoutEffect, useMemo } from "react";
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
import { overviewWorld } from "./overviewSceneData";

const STATES = ["backlog", "projects", "home-record", "record"] as const;
type OverviewSceneState = (typeof STATES)[number];
export type OverviewSceneParams = { state: OverviewSceneState; theme: "light" | "dark"; palette: "hide" | "neutral"; language: "ko" | "en" };

export function overviewSceneParams(params: URLSearchParams): OverviewSceneParams {
  const state = params.get("state") ?? "backlog";
  if (!(STATES as readonly string[]).includes(state)) throw new Error(`Unknown overview scene state ${state}`);
  const palette = params.get("palette") ?? "hide";
  if (palette !== "hide" && palette !== "neutral") throw new Error(`Unknown overview scene palette ${palette}`);
  const language = params.get("lang") ?? "ko";
  if (language !== "ko" && language !== "en") throw new Error(`Unknown overview scene language ${language}`);
  return { state: state as OverviewSceneState, theme: params.get("theme") === "light" ? "light" : "dark", palette, language };
}

export function OverviewScene({ state, theme, palette, language }: OverviewSceneParams) {
  useFollowInterfaceLanguage(language);
  const world = useMemo(() => overviewWorld(Date.now()), []);
  const actions = useMemo(() => createActions(() => true), []);
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
        device_id: "local", path: checkout.path, tool: null, last_tool: "prs", prs: { only_checkout: false }, pinned: true, tree: false, lane_width: null, tree_width: null, lane_called: 0, lane_calls: 0,
        layout: { root: { area: { id: "a1", active: null, displays: [] } }, active_area: "a1", limits: { areas: 6, depth: 3, displays: 32 }, display_count: 0 },
        agent_layout: { root: { area: { id: "g1", active: tabId, displays: [{ id: tabId }] } }, active_area: "g1", canvases: {}, limits: { areas: 6, depth: 3, displays: 16 }, display_count: 1 },
      } as never;
      rest.ui_state = { ...rest.ui_state, detail: { items: [{ kind: "record", session: world.recordSession }], shown: true } } as never;
    }
    useShellStore.setState({ rest, agents: world.agents, connection: "live", sessionRecord });
    useUiStore.setState({
      screen: state === "record" ? { kind: "workspace" } : { kind: "main" },
      overviewOpen: false,
      mainView: state === "projects" ? "projects" : "backlog",
      overview: { ...OVERVIEW_ENTRY, panel: state === "home-record" ? { kind: "record", session: world.recordSession } : null },
      quickAdd: state === "backlog" ? { returnFocus: null } : null,
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
      </div>
    </TooltipProvider>
  );
}
