// The right lane (PRD right-lane B25) in the production Workspace over invented
// local data: each `state` the approved boards draw, from the closed rail to a
// narrow window whose pinned lane opens over the agents. No daemon is used.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { Sidebar } from "../sidebar";
import type { EditorDocumentSnapshot, EditorTabSnapshot, ViewDisplaySnapshot } from "../snapshot";
import { useShellStore, type DirectoryList, type Documents } from "../store";
import { TitleRow } from "../TitleRow";
import { useUiStore } from "../ui";
import type { Tool } from "../workspace";
import { WorkspaceScreen } from "../WorkspaceScreen";
import { REFERENCE_FOLDS, sidebarScene } from "./sceneData";
import { sceneParams, type SceneParams } from "./SidebarScene";

const STATES = ["closed", "overlay", "pinned", "tree-overlay", "tree-only", "expanded", "prs", "narrow"] as const;
type LaneState = (typeof STATES)[number];
type Lane = { tool: Tool | null; pinned: boolean; tree: boolean; documents: boolean; expanded: boolean };

const LANES: Record<LaneState, Lane> = {
  closed: { tool: null, pinned: false, tree: false, documents: true, expanded: false },
  overlay: { tool: "files", pinned: false, tree: false, documents: true, expanded: false },
  pinned: { tool: "files", pinned: true, tree: false, documents: true, expanded: false },
  "tree-overlay": { tool: "files", pinned: false, tree: true, documents: true, expanded: false },
  "tree-only": { tool: "files", pinned: true, tree: false, documents: false, expanded: false },
  expanded: { tool: "files", pinned: false, tree: true, documents: true, expanded: true },
  prs: { tool: "prs", pinned: true, tree: false, documents: true, expanded: false },
  narrow: { tool: "files", pinned: true, tree: false, documents: true, expanded: false },
};

const REVIEW = `# 설정 화면 검토

## 1. 공간

View를 열어도 왼쪽 터미널의 폭과 pane 배치는 그대로다. 핀을 켜면 그때 한 번만 자리를 나눈다.

## 2. 찾기

파일은 위 경로를 눌러 트리를 켜고 찾는다. 트리는 끌 때까지 남고, View가 그만큼 넓어져 문서는 작아지지 않는다.

## 3. 읽기

길게 읽을 땐 확대한다. 돌아오면 이전 배치가 그대로 있다.
`;

export function rightLaneSceneParams(params: URLSearchParams): SceneParams & { state: LaneState } {
  const state = params.get("state") ?? "closed";
  if (!(STATES as readonly string[]).includes(state)) throw new Error(`Unknown right lane state ${state}`);
  return { ...sceneParams(params), state: state as LaneState };
}

export function RightLaneScene({ theme, scale, content, state }: SceneParams & { state: LaneState }) {
  const lane = LANES[state];
  const fixture = useMemo(() => {
    const scene = sidebarScene(content, REFERENCE_FOLDS, Date.now());
    const workspace = scene.rest.navigator!.workspaces!.find((project) => project.id === "hide")!;
    const checkout = workspace.checkouts[0]!;
    for (const project of scene.rest.navigator!.workspaces!) for (const row of project.checkouts) if (row !== checkout) row.tabs = [];
    checkout.tabs = checkout.tabs.slice(0, 1);
    const root = checkout.path;
    const file = (relative: string, label: string, id: string): EditorTabSnapshot => ({ id, workspace_id: workspace.id, checkout_id: checkout.id, path: `${root}/${relative}`, label, kind: "file", diff_committed: null, markdown_live: true, wrap: true, dirty: false, preview: false });
    const tabs = [file("docs/설정검토.md", "설정검토.md", "t1"), file("docs/작업계획.md", "작업계획.md", "t2")];
    const display = (tab: EditorTabSnapshot, id: string): ViewDisplaySnapshot => ({ id, tab_id: tab.id, path: tab.path, label: tab.label, kind: "file", committed: null, preview: false, state: "open", reason: null });
    const displays = lane.documents ? [display(tabs[0]!, "d1"), display(tabs[1]!, "d2")] : [];
    const document: EditorDocumentSnapshot = { path: tabs[0]!.path, language: "markdown", document_kind: "markdown", contents_utf8: REVIEW, revision: "sha256:0", dirty: false, readonly_reason: null, conflict: null, save: null };
    const documents: Documents = lane.documents ? { t1: document } : {};
    const listing = (folder: string, entries: [string, boolean][]): DirectoryList => ({ kind: "file_list", root_path: folder, truncated: false, entries: entries.map(([name, is_directory]) => ({ name, path: `${folder}/${name}`, is_directory })) });
    // A project's usual kinds of folder and file, three levels deep, so the
    // tree's marks and indent guides are drawn as a real checkout shows them.
    const listings = {
      [root]: listing(root, [[".github", true], ["docs", true], ["web", true], [".gitignore", false], ["Cargo.toml", false], ["package.json", false], ["README.md", false]]),
      [`${root}/docs`]: listing(`${root}/docs`, [["설정검토.md", false], ["작업계획.md", false]]),
      [`${root}/web`]: listing(`${root}/web`, [["src", true], ["tsconfig.json", false], ["vite.config.ts", false]]),
      [`${root}/web/src`]: listing(`${root}/web/src`, [["components", true], ["SettingsPanel.tsx", false], ["SettingsPanel.test.tsx", false], ["lane.css", false]]),
    };
    scene.rest.navigator!.focused_workspace_id = workspace.id;
    scene.rest.navigator!.focused_checkout_id = checkout.id;
    scene.rest.ui_state = { ...scene.rest.ui_state!, expanded_paths: Object.keys(listings), selected_path: lane.documents ? tabs[0]!.path : null };
    scene.rest.workspace_view = {
      device_id: "local", path: root, tool: lane.tool, last_tool: lane.tool, prs: { only_checkout: false }, pinned: lane.pinned, tree: lane.tree, lane_width: null, panel_width: null, tree_width: null, lane_called: 0, lane_calls: 0,
      layout: { root: { area: { id: "a1", active: displays[0]?.id ?? null, displays } }, active_area: "a1", limits: { areas: 4, depth: 3, displays: 32 }, display_count: displays.length },
    };
    return { scene, editor: { tabs: lane.documents ? tabs : [], active_tab_id: lane.documents ? "t1" : null }, documents, listings };
  }, [content, lane]);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest: fixture.scene.rest, agents: fixture.scene.agents, connection: "live", editor: fixture.editor, documents: fixture.documents, listings: fixture.listings });
    useUiStore.setState({ screen: { kind: "workspace" }, laneShown: lane.tool !== null, laneExpanded: lane.expanded });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
  }, [fixture, lane, theme, scale]);
  return <TooltipProvider><div className="flex h-full flex-col bg-background text-foreground" data-gallery-scene="right-lane" data-lane-state={state}><TitleRow actions={actions}/><div className="flex min-h-0 flex-1"><Sidebar actions={actions}/><WorkspaceScreen actions={actions}/></div></div></TooltipProvider>;
}
