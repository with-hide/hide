// The ⌘N start panel drawn by the production component over invented snapshot
// data: the where row over the text, the agent and Start under it.
// Starting requires the real daemon; this scene only shows the panel.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { StartPanelHost } from "../StartPanel";
import { useStartPanel } from "../startDraft";
import { worktreeKey } from "../startTargets";
import { useShellStore } from "../store";
import { PALETTE_HIDE_CLASS } from "../theme";
import { useUiStore } from "../ui";
import { REFERENCE_FOLDS, sidebarStatusAxisScene } from "./sceneData";
import { sceneParams } from "./SidebarScene";

const STATES = ["default", "written", "new-branch"] as const;

export function StartPanelScene({ params }: { params: URLSearchParams }) {
  const { theme, scale, content } = sceneParams(params);
  const state = params.get("state") ?? "default";
  if (!(STATES as readonly string[]).includes(state)) throw new Error(`Unknown start panel state ${state}`);
  const palette = params.get("palette") ?? "hide";
  if (palette !== "hide" && palette !== "neutral") throw new Error(`Unknown start panel palette ${palette}`);
  const scene = useMemo(() => {
    const fixture = sidebarStatusAxisScene(content, REFERENCE_FOLDS, Date.now());
    const navigator = fixture.rest.navigator!;
    const project = navigator.workspaces![0]!;
    navigator.focused_workspace_id = project.id;
    navigator.focused_checkout_id = project.checkouts[0]!.id;
    navigator.focused_device_id = "local";
    fixture.rest.ui_state = { ...fixture.rest.ui_state, agent_start: { kind: "claude", models: {} } };
    return fixture;
  }, [content]);
  const actions = useMemo(() => createActions(() => false), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest: scene.rest, agents: scene.agents, connection: "live" });
    useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false });
    const panel = useStartPanel.getState();
    panel.open(false);
    panel.setText(state === "written" ? (content === "long" ? "로그인 화면의 예외 상황을 모두 찾아서 정리하고, 재현 순서와 함께 고칠 방법을 제안해 줘" : "로그인 예외 상황 점검해 줘") : "");
    if (state === "new-branch") {
      const project = scene.rest.navigator!.workspaces![0]!;
      panel.setTarget(worktreeKey(project.device_id ?? "local", project.path));
    }
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
    root.classList.toggle(PALETTE_HIDE_CLASS, palette === "hide");
    root.style.setProperty("--interface-scale", String(scale));
  }, [scene, theme, scale, state, palette, content]);
  return <TooltipProvider><div className="h-full min-w-0 bg-background text-foreground" data-gallery-scene="start-panel" data-start-scene-state={state}><StartPanelHost actions={actions} /></div></TooltipProvider>;
}
