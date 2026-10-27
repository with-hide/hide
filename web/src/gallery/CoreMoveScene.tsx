// The core move's screens on the gallery's synthetic scenes (PRD
// core-host-node-move B2 to B5, B10, B16, W1 to W3): `core-move` is the
// shipped Settings › Devices tab with its ⋯ menus and the move dialog over it,
// `core-move-window` the shell's real strip and Sidebar with the device rail.
// Each state is set up from the address, so the design review captures it as
// the operator would see it; the scene marks itself ready once a menu it
// opens is open. One scene fills one document, because the stores it seeds are
// the app's singletons.

import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import { createActions } from "../actions";
import { ConnectionBadge } from "../badge";
import { TooltipProvider } from "../components/ui/tooltip";
import { CoreMoveDialog, requestOfView } from "../settings/CoreMoveDialog";
import { clientI18n } from "../i18n/translator";
import { DevicesTab } from "../settings/DevicesTab";
import { Sidebar } from "../sidebar";
import { useShellStore } from "../store";
import { OVERVIEW_ENTRY, useUiStore } from "../ui";
import { CORE_MOVE_STATES, CORE_WINDOW_STATES, coreMoveRest, DIALOG_VIEWS, GIVE_UP_STATES, MENU_STATES, NODE_DEVICE, sideOf, WINDOW_STATES, type CoreMoveState } from "./coreMoveSceneData";
import type { SceneContent } from "./sceneData";

export type CoreMoveSceneKind = "core-move" | "core-move-window";
export type CoreMoveSceneParams = { scene: CoreMoveSceneKind; theme: "light" | "dark"; state: CoreMoveState; content: SceneContent; language: "ko" | "en" };

export function coreMoveSceneParams(scene: CoreMoveSceneKind, params: URLSearchParams): CoreMoveSceneParams {
  const known: readonly string[] = scene === "core-move" ? CORE_MOVE_STATES : CORE_WINDOW_STATES;
  // At rest each scene shows its first state.
  const state = params.get("state") ?? known[0]!;
  if (!known.includes(state)) throw new Error(`Unknown ${scene} scene state ${state}`);
  const content = params.get("content") ?? "reference";
  if (content !== "reference" && content !== "long") throw new Error(`Unknown scene content ${content}`);
  // Korean, as the approved bundle draws it, unless the address asks for English.
  return { scene, theme: params.get("theme") === "light" ? "light" : "dark", state: state as CoreMoveState, content, language: params.get("lang") === "en" ? "en" : "ko" };
}

export function CoreMoveScene({ scene, theme, state, content, language }: CoreMoveSceneParams) {
  const side = sideOf(state);
  const fixture = useMemo(() => {
    const next = coreMoveRest(side, content, Date.now());
    if (state === "device-kit-failed") {
      const device = next.rest.navigator!.devices!.find((device) => device.id === NODE_DEVICE)!;
      device.state = "unavailable";
      device.problem = "device_unavailable";
      device.message = "Herdr was not found";
      device.host = {
        ...device.host!, platform: "macos-x86_64", state: "unavailable",
        message: "Device kit download failed: This local/main Hide build has no matching device release; install a released app or provide local device artifacts. Retry the device connection",
      };
      const status = next.rest.status!.remote!.find((row) => row.target_id === NODE_DEVICE)!;
      status.state = "unavailable";
      status.herdr_version = null;
    }
    return next;
  }, [side, content, state]);
  const actions = useMemo(() => createActions(() => true), []);
  const [ready, setReady] = useState(false);

  useLayoutEffect(() => {
    // A node's window names its machine in the address, as `hide connect` does.
    window.history.replaceState(null, "", side === "node" ? `${window.location.pathname}${window.location.search}#node=${NODE_DEVICE}` : `${window.location.pathname}${window.location.search}`);
    const shown = WINDOW_STATES[state];
    useShellStore.setState({ rest: fixture.rest, agents: fixture.agents, connection: shown?.connection ?? "live", coreMove: shown?.move ?? DIALOG_VIEWS[state] ?? null, coreMoveRefusal: null, coreLink: shown?.link ?? null });
    useUiStore.setState({ screen: { kind: "main" }, overview: { ...OVERVIEW_ENTRY, projectId: "herdr-ide" } });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [fixture, side, state, theme]);

  useLayoutEffect(() => {
    void clientI18n.changeLanguage(language);
    document.documentElement.lang = language;
  }, [language]);

  // A menu state opens that device's ⋯ menu from the keyboard and points at its core item, as the bundle draws it.
  useEffect(() => {
    const menu = MENU_STATES[state];
    if (!menu) {
      setReady(true);
      return;
    }
    let frame = 0;
    const open = () => {
      const trigger = document.querySelector<HTMLElement>(`[data-device-menu="${menu.device}"]`);
      const content = document.querySelector(`[data-device-menu-content="${menu.device}"]`);
      if (trigger && !content) trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      const item = document.querySelector<HTMLElement>(menu.item);
      if (!content || !item) {
        frame = requestAnimationFrame(open);
        return;
      }
      if (!item.hasAttribute("data-disabled")) item.focus();
      setReady(true);
    };
    frame = requestAnimationFrame(open);
    return () => cancelAnimationFrame(frame);
  }, [state]);

  const shownView = DIALOG_VIEWS[state];
  const dialog = shownView ? <CoreMoveDialog request={requestOfView(shownView)} asked={null} giveUp={GIVE_UP_STATES.has(state)} actions={actions} onClose={() => {}} onRequest={() => {}} /> : null;
  return (
    <TooltipProvider>
      <div className="flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground" data-gallery-scene={scene} data-core-move-scene-ready={ready ? state : undefined}>
        {scene === "core-move" ? (
          <div className="min-h-0 flex-1 overflow-hidden px-xl py-lg">
            <DevicesTab actions={actions} />
            {dialog}
          </div>
        ) : (
          <>
            <ConnectionBadge actions={actions} />
            <div className="flex min-h-0 flex-1">
              <Sidebar actions={actions} />
              <main className="min-w-0 flex-1 bg-background" />
            </div>
          </>
        )}
      </div>
    </TooltipProvider>
  );
}
