// Approved Group tabs states rendered by the production workspace and sidebar.
// Invented snapshot data only; starting an agent requires the real daemon.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import type { AgentGroup } from "../agentLayout";
import { TooltipProvider } from "../components/ui/tooltip";
import { Sidebar } from "../sidebar";
import type { Tab } from "../snapshot";
import { useShellStore } from "../store";
import { TitleRow } from "../TitleRow";
import { useUiStore } from "../ui";
import { WorkspaceScreen } from "../WorkspaceScreen";
import { REFERENCE_FOLDS, sidebarStatusAxisScene } from "./sceneData";
import { sceneParams } from "./SidebarScene";

const STATES = ["work", "group", "tab", "narrow", "plain", "plain-start"] as const;
function tabId(tab: Tab): string {
  if (tab.id === null) throw new Error("Group scene requires a real tab identity");
  return tab.id;
}

export function GroupTabsScene({ params }: { params: URLSearchParams }) {
  const { theme, scale, content } = sceneParams(params);
  const state = params.get("state") ?? "group";
  if (!(STATES as readonly string[]).includes(state)) throw new Error(`Unknown Group tabs state ${state}`);
  const scene = useMemo(() => {
    const fixture = sidebarStatusAxisScene(content, REFERENCE_FOLDS, Date.now());
    const navigator = fixture.rest.navigator!;
    const project = navigator.workspaces![0]!;
    const checkout = project.checkouts[0]!;
    const review = checkout.tabs[2]!;
    const ungrouped = { ...review, id: "hide:main:t4", sidebar: { number: 4, manual_name: null, named_by_operator: false }, panes: review.panes.slice(1) };
    review.panes = review.panes.slice(0, 1);
    checkout.tabs.push(ungrouped);
    const plain = state === "plain" || state === "plain-start";
    const draft = state === "tab" || state === "narrow" || state === "plain-start";
    const empty = state === "group";
    const groups: AgentGroup[] = [{ id: null, name: null, tabs: plain ? checkout.tabs.map(tabId) : [ungrouped.id] }];
    const names = ["구현", "테스트", "리뷰"];
    checkout.tabs.slice(0, 3).forEach((tab, index) => {
      tab.sidebar = { number: index + 1, manual_name: names[index]!, named_by_operator: true };
    });
    checkout.strip = checkout.tabs.map(tab => ({ id: `strip:${tabId(tab)}`, kind: "herdr", source_id: tabId(tab), label: tab.sidebar?.manual_name ?? "탭 4", preview: false }));
    if (!plain) groups.push({ id: "login", name: content === "long" ? "로그인 개편과 여러 작업의 진행 상황을 함께 확인하는 긴 그룹 이름" : "로그인 개편", tabs: checkout.tabs.slice(0, 3).map(tabId) }, { id: "release", name: "릴리스 준비", tabs: [] });
    const selected = plain ? null : empty ? "release" : "login";
    const selectedGroup = groups.find(group => group.id === selected)!;
    if (draft) selectedGroup.tabs.push("draft-example");
    const active = draft ? "draft-example" : selectedGroup.tabs[0] ?? null;
    const drafts = draft ? ["draft-example"] : [];
    checkout.agent_groups = { selected, groups, drafts };
    checkout.active_tab_id = active;
    const titles = ["로그인 화면 구현", "인증 API 구현", "로그인 예외 상황 점검", "변경 사항 검토", "프로젝트 구조 살펴보기"];
    fixture.agents.forEach((agent, index) => { agent.identity_label = content === "long" ? `${titles[index]} · 긴 한국어 작업 설명과 진행 상태를 확인합니다` : titles[index]!; });
    navigator.focused_workspace_id = project.id;
    navigator.focused_checkout_id = checkout.id;
    navigator.focused_device_id = "local";
    fixture.rest.ui_state = { ...fixture.rest.ui_state, sidebar_width: 264, device_rail_visible: false, left_sidebar_visible: true, agent_start: { kind: "claude", models: {} } };
    fixture.rest.pane_layouts = checkout.tabs.map(tab => ({ workspace_id: project.id, tab_id: tabId(tab), focused_pane_id: tab.panes[0]!.id, zoomed: false, root: tab.panes.length === 1 ? { type: "pane", pane_id: tab.panes[0]!.id } : { type: "split", direction: "right", ratio: 0.5, first: { type: "pane", pane_id: tab.panes[0]!.id }, second: { type: "pane", pane_id: tab.panes[1]!.id } } }));
    fixture.rest.workspace_view = {
      device_id: "local", path: checkout.path, tool: null, last_tool: null, prs: { only_checkout: false }, pinned: false, tree: false,
      lane_width: null, panel_width: null, tree_width: null, lane_called: 0, lane_calls: 0,
      layout: { root: { area: { id: "v1", active: null, displays: [] } }, active_area: "v1", display_count: 0, limits: { areas: 4, depth: 3, displays: 32 } },
      agent_layout: { root: { area: { id: "a1", active, displays: selectedGroup.tabs.map(id => ({ id })) } }, active_area: "a1", display_count: selectedGroup.tabs.length, limits: { areas: 6, depth: 3, displays: 64 }, canvases: {}, group_id: selected, groups, drafts, can_undo_group: false },
    };
    return fixture;
  }, [content, state]);
  const actions = useMemo(() => createActions(() => false), []);
  useLayoutEffect(() => {
    const focusedPaneId = state === "work" || state === "plain" ? "a1" : null;
    useShellStore.setState({ rest: scene.rest, agents: scene.agents, connection: "live", focusedPaneId });
    useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false, laneShown: false, laneExpanded: false });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
  }, [scene, theme, scale, state]);
  return <TooltipProvider><div className="flex h-full min-w-0 flex-col bg-background text-foreground" data-gallery-scene="group-tabs" data-group-scene-state={state}><TitleRow actions={actions} /><div className="flex min-h-0 min-w-0 flex-1"><Sidebar actions={actions} /><WorkspaceScreen actions={actions} /></div></div></TooltipProvider>;
}
