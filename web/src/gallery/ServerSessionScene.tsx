// Production screens over invented local data. No daemon or provider is used.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { Sidebar } from "../sidebar";
import { useShellStore } from "../store";
import { TitleRow } from "../TitleRow";
import { useUiStore } from "../ui";
import { WorkspaceScreen } from "../WorkspaceScreen";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";
import type { SceneParams } from "./SidebarScene";

export function ServerSessionScene({ scene: kind, theme, scale, content }: SceneParams & { scene: "workspace-servers" }) {
  const fixture = useMemo(() => {
    const scene = sidebarScene(content, REFERENCE_FOLDS, Date.now());
    const workspace = scene.rest.navigator!.workspaces!.find((project) => project.id === "hide")!;
    const checkout = workspace.checkouts[0]!;
    const tab = checkout.tabs[0]!;
    const pane = tab.panes[0]!;
    for (const project of scene.rest.navigator!.workspaces!) for (const row of project.checkouts) row.tabs = [];
    checkout.tabs = [{...tab, panes:[pane]}];
    pane.servers = [{host:"127.0.0.1",port:3000},{host:"::1",port:5173}];
    scene.agents = scene.agents.filter((row) => row.pane_id === pane.id);
    scene.rest.navigator!.focused_workspace_id = workspace.id;
    scene.rest.navigator!.focused_checkout_id = checkout.id;
    scene.rest.workspace_view = {device_id:"local",path:checkout.path,tool:null,last_tool:null,prs:{only_checkout:false},pinned:false,tree:false,lane_width:null,panel_width:null,tree_width:null,lane_called:0,lane_calls:0};
    scene.rest.status = {...scene.rest.status,server_discovery:{loading:false,failure:null}};
    return {scene};
  }, [content]);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({rest:fixture.scene.rest,agents:fixture.scene.agents,connection:"live"});
    useUiStore.setState({screen:{kind:"workspace"}});
    document.documentElement.classList.toggle("dark",theme === "dark");
    document.documentElement.classList.toggle("light",theme === "light");
    document.documentElement.style.setProperty("--interface-scale",String(scale));
  }, [fixture,theme,scale]);
  // The Workspace's path and icons are the window's top row (PRD sidebar-titlebar-refresh B17).
  return <TooltipProvider><div className="flex h-full flex-col bg-background text-foreground" data-gallery-scene={kind}><TitleRow actions={actions}/><div className="flex min-h-0 flex-1"><Sidebar actions={actions}/><WorkspaceScreen actions={actions}/></div></div></TooltipProvider>;
}
