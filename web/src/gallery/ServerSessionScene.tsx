// Production screens over invented local data. No daemon or provider is used.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { Sidebar } from "../sidebar";
import { useShellStore } from "../store";
import { TitleRow } from "../TitleRow";
import { useUiStore } from "../ui";
import { WorkspaceScreen, WorkspaceTitle } from "../WorkspaceScreen";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";
import type { SceneParams } from "./SidebarScene";

export function ServerSessionScene({ scene: kind, theme, scale, content, state = "rest" }: SceneParams & { scene: "workspace-servers" | "checkout-git" }) {
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
    if (kind === "checkout-git") {
      const head = "a".repeat(40);
      workspace.label = content === "long" ? "한글과 English가 섞인 매우 긴 프로젝트 이름" : "herdr-ide";
      checkout.branch = content === "long" ? "feature/매우-긴-한글과-English-checkout-name" : "main";
      workspace.is_git = state !== "folder";
      workspace.is_home = state === "home";
      scene.rest.workspace_view.git = {
        state: state === "loading" ? "loading" : state === "unavailable" ? "unavailable" : state === "stale" ? "stale" : "fresh", reason: null,
        facts: ["loading","unavailable"].includes(state) ? null : {head_sha: state === "unborn" ? null : head,subject:content === "long" ? "매우 긴 커밋 제목과 English commit subject that needs truncation without hiding the path and file count" : "탭 전환 시 포커스 유지",commit_unix_seconds:Math.floor(Date.now()/1000)-720,changed_files:state === "clean" ? 0 : 5,conflicts:state === "conflict" ? 2 : 0,upstream:"origin/main",upstream_state:state === "no-upstream" ? "none" : state === "gone" ? "gone" : "tracked",ahead:0,behind:state === "clean" ? 0 : 3,last_fetch_unix_ms:Date.now()-3600000,origin_url:"https://github.com/example/fixture.git"},
        ci:{sha:head,state:state === "pending" || state === "failed" || state === "none" || state === "unknown" ? state : "passing",checked_at_unix_ms:Date.now(),url:null},
      };
    }
    scene.rest.status = {...scene.rest.status,server_discovery:{loading:false,failure:null}};
    return {scene};
  }, [content,kind,state]);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({rest:fixture.scene.rest,agents:fixture.scene.agents,connection:"live"});
    useUiStore.setState({screen:{kind:"workspace"}});
    document.documentElement.classList.toggle("dark",theme === "dark");
    document.documentElement.classList.toggle("light",theme === "light");
    document.documentElement.style.setProperty("--interface-scale",String(scale));
  }, [fixture,theme,scale]);
  // The Workspace's path and icons are the window's top row (PRD sidebar-titlebar-refresh B17).
  return <TooltipProvider><div className="flex h-full flex-col bg-background text-foreground" data-gallery-scene={kind}>{kind === "checkout-git" ? <div className="flex h-(--size-tab-strip) bg-sidebar"><WorkspaceTitle actions={actions}/></div> : <><TitleRow actions={actions}/><div className="flex min-h-0 flex-1"><Sidebar actions={actions}/><WorkspaceScreen actions={actions}/></div></>}</div></TooltipProvider>;
}
