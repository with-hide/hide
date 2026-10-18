// The production sidebar on the gallery's synthetic scene (PRD
// design-review-workflow D-04, B3, B4). This is the shell's own `Sidebar` and
// `createActions`, fed by `sidebarScene` instead of a daemon: a fold the
// operator clicks goes through the real action, and `applyEvent` answers it
// the way the core does. One scene fills one document, because the stores it
// seeds are the app's singletons.

import { useLayoutEffect, useMemo, useState } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { Sidebar } from "../sidebar";
import { TitleRow } from "../TitleRow";
import { SettingsGate } from "../SettingsSheet";
import { useShellStore } from "../store";
import { OVERVIEW_ENTRY, useUiStore } from "../ui";
import { applyEvent, OFFLINE_DEVICE, REFERENCE_FOLDS, sidebarScene, sidebarStatusAxisScene, type SceneContent, type SceneDevices, type SceneFolds, type SceneFront } from "./sceneData";

/** What one scene document shows; every value comes from its query string. */
export type SceneParams = {
  axis?: boolean;
  state?: string;
  theme: "light" | "dark";
  /** The sidebar's width in CSS pixels, seeded as the core's `ui_state.sidebar_width`; absent, the sidebar's default. */
  width: number | null;
  /** The interface text scale the Appearance font size sets (`--interface-scale`); the sidebar itself does not follow it (PRD sidebar-typography D-05). */
  scale: number;
  content: SceneContent;
  /** One device, the two-device fixture, or that fixture with a busy `mini`; the rail shows in all three. */
  devices: SceneDevices;
  /** What is in front once the scene opens; a rail tile changes it. */
  front: SceneFront;
  /** `hidden` seeds the core's `ui_state.device_rail_visible` as false: the name on the top line is the device menu. */
  rail: "shown" | "hidden";
};

export function sceneParams(params: URLSearchParams): SceneParams {
  const width = params.get("width");
  const scale = Number(params.get("scale") ?? "1");
  const content = params.get("content") ?? "reference";
  if (width !== null && !(Number(width) > 0)) throw new Error(`Scene width must be a positive number, got ${width}`);
  if (!(scale > 0)) throw new Error(`Scene scale must be a positive number, got ${params.get("scale")}`);
  if (content !== "reference" && content !== "long") throw new Error(`Unknown scene content ${content}`);
  const devices = params.get("devices") ?? "one";
  if (devices !== "one" && devices !== "two" && devices !== "busy") throw new Error(`Unknown scene devices ${devices}`);
  const front = params.get("front") ?? "local";
  if (front !== "local" && front !== "mini" && front !== "offline") throw new Error(`Unknown scene front ${front}`);
  if (devices === "one" && front !== "local") throw new Error(`Scene front ${front} needs devices=two`);
  const rail = params.get("rail") ?? "shown";
  if (rail !== "shown" && rail !== "hidden") throw new Error(`Unknown scene rail ${rail}`);
  return { theme: params.get("theme") === "light" ? "light" : "dark", width: width === null ? null : Number(width), scale, content, devices, front, rail, state: params.get("state") ?? "rest" };
}

export function SidebarScene({ theme, width, scale, content, devices, front, rail, axis = false }: SceneParams) {
  const [folds, setFolds] = useState<SceneFolds>({ ...REFERENCE_FOLDS, frontDevice: front === "mini" ? "mini" : front === "offline" ? OFFLINE_DEVICE : "local" });
  const actions = useMemo(
    () =>
      createActions((event) => {
        setFolds((current) => {
          const next = applyEvent(current, event);
          if (!next) console.info(`gallery scene: ${event.kind} is not modelled here`);
          return next ?? current;
        });
      }),
    [],
  );

  // Seed the app's stores before the first paint, and again on every fold.
  const scene = useMemo(() => axis ? sidebarStatusAxisScene(content, folds, Date.now()) : sidebarScene(content, folds, Date.now(), devices), [content, folds, devices, axis]);
  useLayoutEffect(() => {
    const sized = width === null ? scene.rest : { ...scene.rest, ui_state: { ...scene.rest.ui_state, sidebar_width: width } };
    const rest = { ...sized, ui_state: { ...sized.ui_state, device_rail_visible: rail === "shown" } };
    useShellStore.setState({ rest, agents: scene.agents, connection: "live", ...(axis ? { focusedPaneId: "a1" } : {}) });
  }, [scene, width, rail, axis]);

  useLayoutEffect(() => {
    useUiStore.setState({ screen: axis ? { kind: "workspace" } : { kind: "main" }, overview: { ...OVERVIEW_ENTRY, projectId: "hide" }, overviewOpen: false });
  }, [front, axis]);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
    if (scale === 1) root.style.removeProperty("--interface-scale");
    else root.style.setProperty("--interface-scale", String(scale));
  }, [theme, scale]);

  return (
    <TooltipProvider>
      <div className="flex h-full flex-col bg-background text-foreground" data-gallery-scene="projects-sidebar" data-scene-content={content} data-scene-devices={devices} data-scene-front={front} data-scene-rail={rail}>
        {/* The device name sits in the window's top row over the sidebar (PRD sidebar-titlebar-refresh B17). */}
        <TitleRow actions={actions} />
        <div className="flex min-h-0 flex-1">
          <Sidebar actions={actions} />
        </div>
      </div>
      <SettingsGate actions={actions} />
    </TooltipProvider>
  );
}
