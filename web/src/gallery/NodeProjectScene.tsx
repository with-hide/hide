// A MacBook project in the device Overview after the core moved (PRD
// core-host-node-move D-29, overview-device-tasks D-02) on the gallery's
// synthetic data: the shipped Overview on the MacBook with its 백로그 tab over
// a GitHub or a Local source, and the disk cleanup sheet its Projects tab
// opens. Each state is set up from the address; one scene fills one document,
// because the stores it seeds are the app's singletons.

import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { clientI18n } from "../i18n/translator";
import { OverviewPage } from "../Overview";
import { useShellStore } from "../store";
import { OVERVIEW_ENTRY, useUiStore } from "../ui";
import { WorkspaceDialogs } from "../WorkspaceDialogs";
import { NODE_DEVICE } from "./coreLinkSceneData";
import { NODE_PROJECT, nodeProjectRest } from "./nodeProjectSceneData";

export const NODE_PROJECT_STATES = ["backlog-github", "backlog-local", "cleanup"] as const;
export type NodeProjectState = (typeof NODE_PROJECT_STATES)[number];
export type NodeProjectSceneParams = { theme: "light" | "dark"; state: NodeProjectState; language: "ko" | "en" };

export function nodeProjectSceneParams(params: URLSearchParams): NodeProjectSceneParams {
  const state = params.get("state") ?? NODE_PROJECT_STATES[0];
  if (!(NODE_PROJECT_STATES as readonly string[]).includes(state)) throw new Error(`Unknown node-project scene state ${state}`);
  return { theme: params.get("theme") === "light" ? "light" : "dark", state: state as NodeProjectState, language: params.get("lang") === "en" ? "en" : "ko" };
}

export function NodeProjectScene({ theme, state, language }: NodeProjectSceneParams) {
  const fixture = useMemo(() => nodeProjectRest(state === "backlog-github" ? "github" : "local", state === "cleanup", "reference", Date.now()), [state]);
  const actions = useMemo(() => createActions(() => true), []);

  useLayoutEffect(() => {
    // The MacBook's window names its machine in the address, as `hide connect` does.
    window.history.replaceState(null, "", `${window.location.pathname}${window.location.search}#node=${NODE_DEVICE}`);
    useShellStore.setState({ rest: fixture.rest, agents: fixture.agents, connection: "live" });
    useUiStore.setState({
      screen: { kind: "main", deviceId: NODE_DEVICE },
      mainView: state === "cleanup" ? "projects" : "backlog",
      overview: { ...OVERVIEW_ENTRY, deviceId: NODE_DEVICE, projectId: state === "cleanup" ? null : NODE_PROJECT },
      workspaceDialog: state === "cleanup" ? { kind: "disk_cleanup", workspaceId: NODE_PROJECT, filter: "done" } : null,
    });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [fixture, state, theme]);

  useLayoutEffect(() => {
    void clientI18n.changeLanguage(language);
    document.documentElement.lang = language;
  }, [language]);

  return (
    <TooltipProvider>
      <div className="flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground" data-gallery-scene="node-project" data-node-project-scene-ready={state}>
        <OverviewPage actions={actions} />
        <WorkspaceDialogs actions={actions} />
      </div>
    </TooltipProvider>
  );
}
