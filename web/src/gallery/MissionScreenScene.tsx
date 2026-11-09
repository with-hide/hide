// The mission screen (PRD mission-screen B1-B23) in the production shell over
// invented data: the sidebar with its `미션` row, and the screen in each state
// it carries. `table` is the device's missions in their four groups over two
// projects; `filtered` narrows them to one project's chip; `detail` is the list
// beside a mission's detail; `closed-all` opens 끝남 whole; `empty`,
// `loading` and `failed` stand in for the rows; `offline` is a device that does
// not answer. `window` sets the window's width, so a narrow one stacks the list above
// the detail. No daemon is used.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { MissionScreen } from "../MissionScreen";
import { Sidebar } from "../sidebar";
import { useShellStore } from "../store";
import { TitleRow } from "../TitleRow";
import { MISSION_ENTRY, useUiStore, type MissionPlace } from "../ui";
import { missionScreenMissions } from "./missionScreenSceneData";
import { OFFLINE_DEVICE, REFERENCE_FOLDS, sidebarScene } from "./sceneData";
import { sceneParams, type SceneParams } from "./SidebarScene";

const STATES = ["table", "filtered", "detail", "closed-all", "empty", "loading", "failed", "offline"] as const;
type MissionScreenState = (typeof STATES)[number];

/** The second project the missions spread over, so the table shows its chips. */
const OTHER = "sasu";

type MissionScreenParams = SceneParams & { state: MissionScreenState; window: number | null };

export function missionScreenSceneParams(params: URLSearchParams): MissionScreenParams {
  const state = params.get("state") ?? "table";
  if (!(STATES as readonly string[]).includes(state)) throw new Error(`Unknown mission screen state ${state}`);
  const window = params.get("window");
  if (window !== null && !(Number(window) > 0)) throw new Error(`Scene window must be a positive number, got ${window}`);
  return { ...sceneParams(params), state: state as MissionScreenState, window: window === null ? null : Number(window) };
}

export function MissionScreenScene({ theme, scale, content, state, window }: MissionScreenParams) {
  const fixture = useMemo(() => {
    const now = Date.now();
    const scene = sidebarScene(content, REFERENCE_FOLDS, now, state === "offline" ? "two" : "one");
    const projects = scene.rest.navigator!.workspaces!;
    const hide = projects.find((project) => project.id === "hide")!;
    const other = projects.find((project) => project.id === OTHER)!;
    const device = state === "offline" ? OFFLINE_DEVICE : "local";
    const missions = missionScreenMissions(hide.id, hide.checkouts[0]!.path, other.id, other.checkouts[0]!.path, now).map((mission) => ({ ...mission, device_id: device, stale: state === "offline" }));
    scene.rest.missions = state === "empty" ? { items: [] } : state === "failed" ? { items: [], failed: "mission book: unreadable" } : { items: missions };
    if (state === "loading") delete scene.rest.missions;
    const place: MissionPlace = {
      ...MISSION_ENTRY,
      deviceId: device,
      projectId: state === "filtered" ? hide.id : null,
      selected: state === "detail" ? "mission-41" : null,
      closedAll: state === "closed-all",
    };
    return { scene, place };
  }, [content, state]);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest: fixture.scene.rest, agents: fixture.scene.agents, connection: "live" });
    useUiStore.setState({ screen: { kind: "missions" }, missions: fixture.place });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
  }, [fixture, theme, scale]);
  return (
    <TooltipProvider>
      <div style={window ? { width: window } : undefined} className="flex h-full min-w-0 flex-col bg-background text-foreground" data-gallery-scene="mission-screen" data-mission-screen-scene={state}>
        <TitleRow actions={actions} />
        <div className="flex min-h-0 flex-1">
          <Sidebar actions={actions} />
          <main className="flex min-h-0 min-w-0 flex-1 flex-col">
            <MissionScreen actions={actions} />
          </main>
        </div>
      </div>
    </TooltipProvider>
  );
}
