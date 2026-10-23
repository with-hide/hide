// The introduction site's product shots (site/index.html, the hero's tabs and
// the sections under it): the shell's own window, Sidebar, Overview, Project
// tool, browser display and Settings tabs over the invented story of the page's one-night timeline
// (`landingSceneData.ts`). One shot fills one document, because the stores a
// scene seeds are the app's singletons; separately maintained capture tooling
// photographs each in both themes.
//
//   hero       the whole window: the sidebar's Projects list, a lead and the agent asking to merge side by side, and the Project tool
//   tasks      the hero's window with the sidebar on its Tasks list, every task of the device by state
//   quiet      the tasks shot with Agent summaries off: provider names and native questions only
//   prs        the hero's window with the sidebar on its PR list, pull requests by who acts next
//   map        the Overview's work graph: who started whom across the projects, and who waits on you
//   projects   the Overview's Projects tab: each project's sessions, root tasks by state, disk and what to handle
//   team       the hero's window, where the capture opens the lead's child tree from its pane head
//   pr         the hero's window with the card of pull request 431 in the lane
//   record     the hero's window with the lead's session record in the lane
//   conductor  the lead's pane conducting its mission, with the mission's map in the lane
//   attention  the sidebar with the Needs You and Unread groups raised above the project tree
//   plain      the attention shot with Agent summaries off: provider names and native questions only
//   memory     the lead beside a wide Project tool on its Memory tab
//   browser    a Codex worker beside the invoices page it opened and the file it changed
//   core       the device rail beside Settings > Devices and the paired phone

import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import { createActions } from "../actions";
import { hostKey, useBrowserStore } from "../browserViews";
import type { HostBridge } from "../host";
import type { SnapshotRest } from "../snapshot";
import { TooltipProvider } from "../components/ui/tooltip";
import { clientI18n } from "../i18n/translator";
import { MobileTab } from "../MobileTab";
import { OverviewPage } from "../Overview";
import { DevicesTab } from "../settings/DevicesTab";
import { Sidebar } from "../sidebar";
import { useShellStore } from "../store";
import { PALETTE_HIDE_CLASS } from "../theme";
import { feedChunks, terminalFor } from "../terminals";
import { TitleRow } from "../TitleRow";
import { useUiStore } from "../ui";
import { workspaceKey } from "../viewLayout";
import { WorkspaceScreen } from "../WorkspaceScreen";
import { LEAD_PANE, laneItem, withSessions, landingMission, landingPrCard, landingPrLink, landingPrWork, landingRecord, type LandingLane } from "./landingLaneData";
import { BROWSER_DISPLAY, BROWSER_PANE, BROWSER_TERMINAL, HERO_PANES, HERO_TERMINALS, RULE, landingBrowser, landingDevices, landingMap, landingMemory, landingPlainSidebar, landingSidebar, landingUsage, landingWorkspace, withoutHideAi } from "./landingSceneData";
// The invoices page the agent opened, after its click: what the desktop host would show in the browser display.
import invoicesDark from "./landing-invoices-dark.svg?url";
import invoicesLight from "./landing-invoices-light.svg?url";

export const LANDING_SHOTS = ["hero", "tasks", "quiet", "prs", "map", "projects", "attention", "plain", "team", "pr", "record", "conductor", "memory", "browser", "core"] as const;
export type LandingShot = (typeof LANDING_SHOTS)[number];
export type LandingSceneParams = { shot: LandingShot; theme: "light" | "dark" };

export function landingSceneParams(params: URLSearchParams): LandingSceneParams {
  const shot = params.get("shot") ?? LANDING_SHOTS[0];
  if (!(LANDING_SHOTS as readonly string[]).includes(shot)) throw new Error(`Unknown landing scene shot ${shot}`);
  return { shot: shot as LandingShot, theme: params.get("theme") === "light" ? "light" : "dark" };
}

/** The sidebar's width: wide enough that the page's example titles read whole, narrower where Settings needs the room. */
const SIDEBAR_WIDTH: Record<LandingShot, number> = { hero: 320, tasks: 320, quiet: 320, prs: 320, map: 320, projects: 320, attention: 380, plain: 380, team: 320, pr: 300, record: 300, conductor: 300, memory: 320, browser: 300, core: 340 };
/** The shots of a Workspace: the hero's window, and the browser shot's pane beside its View area. */
const WORKSPACE_SHOTS: readonly LandingShot[] = ["hero", "tasks", "quiet", "prs", "team", "pr", "record", "conductor", "memory", "browser"];
/** The shots whose lane shows an item in its 보기 slot. */
const LANE_SHOTS: readonly LandingShot[] = ["pr", "record", "conductor"];
/** The slot's width in a lane shot: room for a card, a record or a map beside the lead. */
const DETAIL_WIDTH: Record<LandingLane, number> = { pr: 620, record: 620, conductor: 720 };

/**
 * A browser display's page is drawn by the desktop host, which a gallery tab
 * does not have; the browser shot stands in a host that keeps no page and
 * answers nothing, so the shell draws the display's toolbar and the still it is
 * given. Every other shot runs as a plain browser tab.
 */
function standInHost(): HostBridge {
  const quiet = (name: string) => (name.startsWith("on") ? () => () => {} : () => Promise.resolve(null));
  const part = <T extends object>(fixed: Record<string, unknown>): T => new Proxy(fixed, { get: (target, name) => (typeof name !== "string" || name === "then" ? undefined : name in target ? target[name] : quiet(name)) }) as T;
  return part<HostBridge>({ kind: "electron", platform: "darwin", browser: part({}), updates: part({}) });
}

export function LandingScene({ shot, theme }: LandingSceneParams) {
  const actions = useMemo(() => createActions(() => true), []);
  const workspace = WORKSPACE_SHOTS.includes(shot);
  const fixture = useMemo(() => {
    const now = Date.now();
    const none = { mobile: null, usage: null, memory: null, editor: null, documents: null, linkPanel: null, sessionRecord: null };
    if (shot === "core") return { ...none, ...landingDevices(now, SIDEBAR_WIDTH.core) };
    if (shot === "attention") return { ...none, ...landingSidebar(now, [], SIDEBAR_WIDTH.attention) };
    if (shot === "plain") return { ...none, ...landingPlainSidebar(now, SIDEBAR_WIDTH.plain) };
    if (shot === "map" || shot === "projects") return { ...none, ...landingMap(now) };
    if ((LANE_SHOTS as readonly string[]).includes(shot)) {
      const lane = shot as LandingLane;
      const base = landingWorkspace(now, SIDEBAR_WIDTH[shot]);
      const scene = { ...base, agents: withSessions(base.agents) };
      const rest = structuredClone(scene.rest);
      // Hide AI on, as the hero's titles say it is, so the record shows its summaries.
      rest.status = { ...rest.status, background_ai: { provider: "claude", providers: [], unavailable_reason: null, enabled: true, chosen: true, agent_summary: true } } as SnapshotRest["status"];
      const project = rest.navigator!.workspaces!.find((row) => row.id === "hide")!;
      const tab = project.checkouts[0]!.tabs[0]!;
      rest.ui_state = { ...rest.ui_state!, detail: { items: [laneItem(lane, scene.agents)], shown: true } };
      rest.workspace_view = { ...rest.workspace_view!, tool: null, detail_width: DETAIL_WIDTH[lane] };
      // The lead alone in its tab, so the slot beside it has the room.
      rest.pane_layouts = [{ workspace_id: project.id, tab_id: tab.id!, focused_pane_id: LEAD_PANE, zoomed: false, root: { type: "pane", pane_id: LEAD_PANE } }];
      if (lane === "conductor") rest.missions = { items: [landingMission(now, scene.agents)] };
      let linkPanel = null;
      if (lane === "pr") {
        project.pull_requests = (project.pull_requests ?? []).map((pull) => (pull.number === 431 ? landingPrWork(pull) : pull));
        rest.pr_work = { link: null, feedback: null, cards: [landingPrCard()] };
        linkPanel = landingPrLink(now, scene.agents, project.pull_requests.find((pull) => pull.number === 431)!);
      }
      const record = lane === "record" ? landingRecord(now, scene.agents) : null;
      return { ...none, rest, agents: scene.agents, usage: landingUsage(now), linkPanel, sessionRecord: record ? { session: record.session, loading: false, failure: null, record } : null };
    }
    if (shot === "browser") {
      if (!window.hideHost) window.hideHost = standInHost();
      return { ...none, ...landingBrowser(now, SIDEBAR_WIDTH.browser) };
    }
    if (shot === "memory") {
      // The lead alone beside a Project tool as wide as the lane shots' slot, so the page can crop the lane at one size.
      const scene = landingWorkspace(now, SIDEBAR_WIDTH.memory);
      const rest = structuredClone(scene.rest);
      const project = rest.navigator!.workspaces!.find((row) => row.id === "hide")!;
      const tab = project.checkouts[0]!.tabs[0]!;
      rest.workspace_view = { ...rest.workspace_view!, panel_width: DETAIL_WIDTH.pr };
      rest.pane_layouts = [{ workspace_id: project.id, tab_id: tab.id!, focused_pane_id: LEAD_PANE, zoomed: false, root: { type: "pane", pane_id: LEAD_PANE } }];
      return { ...none, rest, agents: scene.agents, usage: landingUsage(now), memory: landingMemory(now, "hide") };
    }
    if (shot === "quiet") return { ...none, ...withoutHideAi(landingWorkspace(now, SIDEBAR_WIDTH.quiet, [], "tasks")), usage: landingUsage(now) };
    const lens = shot === "tasks" ? "tasks" : shot === "prs" ? "prs" : "projects";
    return { ...none, ...landingWorkspace(now, SIDEBAR_WIDTH[shot], [], lens), usage: landingUsage(now) };
  }, [shot]);

  useLayoutEffect(() => {
    useShellStore.setState({ rest: fixture.rest, agents: fixture.agents, mobile: fixture.mobile, usage: fixture.usage ? { nodes: [fixture.usage] } : null, memory: { project: fixture.memory }, connection: "live", linkPanel: fixture.linkPanel, sessionRecord: fixture.sessionRecord, ...(fixture.editor ? { editor: fixture.editor, documents: fixture.documents } : {}) });
    useUiStore.setState({ screen: { kind: workspace ? "workspace" : "main" }, overviewOpen: false, laneShown: workspace, projectTab: shot === "memory" ? "memory" : "status", stackFront: "view", ...(shot === "map" ? { mainView: "work" as const } : shot === "projects" ? { mainView: "projects" as const } : {}) });
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
    root.classList.add(PALETTE_HIDE_CLASS);
  }, [fixture, shot, theme, workspace]);

  // The browser display's page: the still the desktop host would hold of it, with the page's own state.
  useEffect(() => {
    if (shot !== "browser") return undefined;
    const view = fixture.rest.workspace_view!;
    const key = hostKey(workspaceKey({ device_id: view.device_id, path: view.path }), BROWSER_DISPLAY);
    const page = { url: "http://localhost:5173/invoices", title: "Invoices", loading: false, canGoBack: true, canGoForward: false, failure: null, zoom: 1 };
    const still = theme === "dark" ? invoicesDark : invoicesLight;
    const put = () => useBrowserStore.setState((store) => ({ pages: { ...store.pages, [key]: page }, stills: { ...store.stills, [BROWSER_DISPLAY]: still } }));
    put();
    return useBrowserStore.subscribe((store) => { if (store.stills[BROWSER_DISPLAY] !== still || store.pages[key] !== page) put(); });
  }, [fixture, shot, theme]);

  // The workspace's terminals show what each agent CLI drew, written as the core
  // streams a pane's output, once each terminal has fitted its pane: a pane
  // keeps no scrollback, so text written into a smaller grid would be lost.
  const [written, setWritten] = useState(false);
  useEffect(() => {
    if (!workspace) return;
    const encode = (text: string) => btoa(String.fromCharCode(...new TextEncoder().encode(text)));
    const screens: Record<string, string> = shot === "browser" ? { [BROWSER_PANE]: BROWSER_TERMINAL } : (LANE_SHOTS as readonly string[]).includes(shot) || shot === "memory" ? { [LEAD_PANE]: HERO_TERMINALS[LEAD_PANE] } : Object.fromEntries(HERO_PANES.map((id) => [id, HERO_TERMINALS[id]]));
    const panes = Object.keys(screens);
    let frame = requestAnimationFrame(function wait() {
      const fitted = panes.every((id) => (terminalFor(id)?.rows ?? 0) > screens[id]!.split("\r\n").length);
      if (!fitted) {
        frame = requestAnimationFrame(wait);
        return;
      }
      feedChunks(panes.map((pane_id) => ({ pane_id, bytes_base64: encode(screens[pane_id]!.replaceAll(RULE, "─".repeat(terminalFor(pane_id)!.cols))) })));
      setWritten(true);
    });
    return () => cancelAnimationFrame(frame);
  }, [shot, workspace]);

  useLayoutEffect(() => {
    void clientI18n.changeLanguage("en");
    document.documentElement.lang = "en";
  }, []);

  return (
    <TooltipProvider>
      <div className="flex h-full flex-col bg-background text-foreground" data-gallery-scene="landing" data-landing-shot={shot} data-landing-terminals={written ? "written" : undefined}>
        <TitleRow actions={actions} />
        <div className="flex min-h-0 flex-1">
          {shot === "map" || shot === "projects" ? null : <Sidebar actions={actions} />}
          {workspace ? <WorkspaceScreen actions={actions} /> : null}
          {shot === "map" || shot === "projects" ? (
            <main className="flex min-h-0 min-w-0 flex-1 flex-col">
              <OverviewPage actions={actions} />
            </main>
          ) : null}
          {shot === "core" ? (
            <main className="min-w-0 flex-1 overflow-hidden px-xl py-lg" data-landing-settings>
              <DevicesTab actions={actions} />
              <div className="mt-xl">
                <MobileTab actions={actions} />
              </div>
            </main>
          ) : null}
        </div>
      </div>
    </TooltipProvider>
  );
}
