// The introduction site's product shots (site/index.html, the pillars): the
// shell's own Sidebar, pane header and Settings tabs over the invented story
// of the page's one-night timeline (`landingSceneData.ts`). One shot fills one
// document, because the stores a scene seeds are the app's singletons;
// `site/scripts/capture-shots.mjs` photographs each in both themes.
//
//   attention  the sidebar with the Needs You and Done groups raised above the project tree
//   agents     a lead with its children across agent CLIs in the tree, and the lead's pane header with its child row
//   core       the device rail beside Settings > Devices and the paired phone

import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { clientI18n } from "../i18n/translator";
import { MobileTab } from "../MobileTab";
import { PaneView } from "../PaneView";
import { DevicesTab } from "../settings/DevicesTab";
import { Sidebar } from "../sidebar";
import { useShellStore } from "../store";
import { TitleRow } from "../TitleRow";
import { useUiStore } from "../ui";
import { landingDevices, landingSidebar } from "./landingSceneData";

export const LANDING_SHOTS = ["attention", "agents", "core"] as const;
export type LandingShot = (typeof LANDING_SHOTS)[number];
/** `narrow` is the phone variant: the key part of the shot alone, at a width a phone shows without sideways scrolling. */
export type LandingSceneParams = { shot: LandingShot; theme: "light" | "dark"; narrow: boolean };

export function landingSceneParams(params: URLSearchParams): LandingSceneParams {
  const shot = params.get("shot") ?? LANDING_SHOTS[0];
  if (!(LANDING_SHOTS as readonly string[]).includes(shot)) throw new Error(`Unknown landing scene shot ${shot}`);
  return { shot: shot as LandingShot, theme: params.get("theme") === "light" ? "light" : "dark", narrow: params.get("narrow") === "1" };
}

/** The sidebar's width: wide enough that the page's example titles read whole, narrower where Settings needs the room. */
const SIDEBAR_WIDTH: Record<LandingShot, number> = { attention: 380, agents: 380, core: 340 };
/** The phone variant of the agents shot is the sidebar tree alone. */
const NARROW_SIDEBAR_WIDTH = 372;

export function LandingScene({ shot, theme, narrow }: LandingSceneParams) {
  const actions = useMemo(() => createActions(() => true), []);
  const fixture = useMemo(() => {
    const now = Date.now();
    if (shot === "core") return { ...landingDevices(now, SIDEBAR_WIDTH.core), pane: null };
    const scene = landingSidebar(now, shot === "agents" ? ["a1"] : [], narrow ? NARROW_SIDEBAR_WIDTH : SIDEBAR_WIDTH[shot]);
    const lead = scene.agents.find((row) => row.pane_id === "a1")!;
    const pane = { ...scene.rest.navigator!.workspaces!.find((row) => row.id === "hide")!.checkouts[0]!.tabs[0]!.panes.find((row) => row.id === "a1")!, identity_label: lead.identity_label };
    // The lead's header carries its child row; nothing else about the pane is staged.
    const rest = { ...scene.rest, terminal: { headers: { a1: { working: true, notice: null, band: null } } } };
    return { ...scene, rest, pane, mobile: null };
  }, [shot, narrow]);

  useLayoutEffect(() => {
    useShellStore.setState({ rest: fixture.rest, agents: fixture.agents, mobile: fixture.mobile, connection: "live" });
    useUiStore.setState({ screen: { kind: "main" }, overviewOpen: false });
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
  }, [fixture, theme]);

  useLayoutEffect(() => {
    void clientI18n.changeLanguage("en");
    document.documentElement.lang = "en";
  }, []);

  return (
    <TooltipProvider>
      <div className="flex h-full flex-col bg-background text-foreground" data-gallery-scene="landing" data-landing-shot={shot} data-landing-narrow={narrow ? "" : undefined}>
        {narrow && shot === "core" ? null : <TitleRow actions={actions} />}
        <div className="flex min-h-0 flex-1">
          {narrow && shot === "core" ? null : <Sidebar actions={actions} />}
          {shot === "agents" && fixture.pane && !narrow ? (
            <main className="min-w-0 flex-1 p-lg">
              <div className="h-full min-w-0 overflow-hidden border border-border" data-landing-pane>
                <PaneView pane={fixture.pane} transport={undefined} focused={false} scale={1} actions={actions} agentKind="claude" markSymbol="going" markTone="" paneCount={1} zoomed={false} />
              </div>
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
