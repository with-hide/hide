// The introduction site's product shots (site/index.html, the hero's tabs and
// the Hide AI section): the shell's own window, Sidebar, Memory tool and
// Settings tabs over the invented story of the page's one-night timeline
// (`landingSceneData.ts`). One shot fills one document, because the stores a
// scene seeds are the app's singletons; `scripts/capture-site-shots.mjs`
// photographs each in both themes.
//
//   hero       the whole window: the sidebar, a lead and the agent asking to merge side by side, and the PRs tool
//   agents     the hero's window with the lead's children unfolded in the tree and listed from its pane header
//   memory     the hero's window with the Memory tool pinned: the Project's cards, one asked about for the docs
//   attention  the sidebar with the Needs You and Unread groups raised above the project tree
//   core       the device rail beside Settings > Devices and the paired phone

import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { clientI18n } from "../i18n/translator";
import { MobileTab } from "../MobileTab";
import { DevicesTab } from "../settings/DevicesTab";
import { Sidebar } from "../sidebar";
import { useShellStore } from "../store";
import { feedChunks, terminalFor } from "../terminals";
import { TitleRow } from "../TitleRow";
import { useUiStore } from "../ui";
import { WorkspaceScreen } from "../WorkspaceScreen";
import { HERO_PANES, HERO_TERMINALS, RULE, landingDevices, landingMemory, landingSidebar, landingWorkspace } from "./landingSceneData";

export const LANDING_SHOTS = ["hero", "agents", "memory", "attention", "core"] as const;
export type LandingShot = (typeof LANDING_SHOTS)[number];
export type LandingSceneParams = { shot: LandingShot; theme: "light" | "dark" };

export function landingSceneParams(params: URLSearchParams): LandingSceneParams {
  const shot = params.get("shot") ?? LANDING_SHOTS[0];
  if (!(LANDING_SHOTS as readonly string[]).includes(shot)) throw new Error(`Unknown landing scene shot ${shot}`);
  return { shot: shot as LandingShot, theme: params.get("theme") === "light" ? "light" : "dark" };
}

/** The sidebar's width: wide enough that the page's example titles read whole, narrower where Settings needs the room. */
const SIDEBAR_WIDTH: Record<LandingShot, number> = { hero: 320, agents: 320, memory: 320, attention: 380, core: 340 };
/** The shots of the whole window, with the lead and the agent asking to merge side by side. */
const WORKSPACE_SHOTS: readonly LandingShot[] = ["hero", "agents", "memory"];

export function LandingScene({ shot, theme }: LandingSceneParams) {
  const actions = useMemo(() => createActions(() => true), []);
  const workspace = WORKSPACE_SHOTS.includes(shot);
  const fixture = useMemo(() => {
    const now = Date.now();
    if (shot === "core") return { ...landingDevices(now, SIDEBAR_WIDTH.core), memory: null };
    if (shot === "attention") return { ...landingSidebar(now, [], SIDEBAR_WIDTH.attention), mobile: null, memory: null };
    const scene = landingWorkspace(now, SIDEBAR_WIDTH[shot], shot === "memory" ? "memory" : "prs", shot === "agents" ? ["a1"] : []);
    return { ...scene, mobile: null, memory: shot === "memory" ? landingMemory(now, "hide") : null };
  }, [shot]);

  useLayoutEffect(() => {
    useShellStore.setState({ rest: fixture.rest, agents: fixture.agents, mobile: fixture.mobile, memory: { project: fixture.memory }, connection: "live" });
    useUiStore.setState({ screen: { kind: workspace ? "workspace" : "main" }, overviewOpen: false, laneShown: workspace });
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
  }, [fixture, theme, workspace]);

  // The workspace's terminals show what each agent CLI drew, written as the core
  // streams a pane's output, once each terminal has fitted its pane: a pane
  // keeps no scrollback, so text written into a smaller grid would be lost.
  const [written, setWritten] = useState(false);
  useEffect(() => {
    if (!workspace) return;
    const encode = (text: string) => btoa(String.fromCharCode(...new TextEncoder().encode(text)));
    let frame = requestAnimationFrame(function wait() {
      const fitted = HERO_PANES.every((id) => (terminalFor(id)?.rows ?? 0) > HERO_TERMINALS[id].split("\r\n").length);
      if (!fitted) {
        frame = requestAnimationFrame(wait);
        return;
      }
      feedChunks(HERO_PANES.map((pane_id) => ({ pane_id, bytes_base64: encode(HERO_TERMINALS[pane_id].replaceAll(RULE, "─".repeat(terminalFor(pane_id)!.cols))) })));
      setWritten(true);
    });
    return () => cancelAnimationFrame(frame);
  }, [workspace]);

  useLayoutEffect(() => {
    void clientI18n.changeLanguage("en");
    document.documentElement.lang = "en";
  }, []);

  return (
    <TooltipProvider>
      <div className="flex h-full flex-col bg-background text-foreground" data-gallery-scene="landing" data-landing-shot={shot} data-landing-terminals={written ? "written" : undefined}>
        <TitleRow actions={actions} />
        <div className="flex min-h-0 flex-1">
          <Sidebar actions={actions} />
          {workspace ? <WorkspaceScreen actions={actions} /> : null}
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
