import { clientI18n } from "../i18n/translator";
import { requireInterfaceLanguage } from "../i18n/locale";
import { StrictMode } from "react";
import type { Root } from "react-dom/client";
import { Gallery, GalleryFrame } from "./Gallery";
import { CmdkScene, cmdkSceneParams } from "./CmdkScene";
import { GALLERY, type Section } from "./manifest";
import { ServerSessionScene } from "./ServerSessionScene";
import { RightLaneScene, rightLaneSceneParams } from "./RightLaneScene";
import { GroupTabsScene } from "./GroupTabsScene";
import { ViewFeedbackScene, viewFeedbackSceneParams } from "./ViewFeedbackScene";
import { sceneParams, SidebarScene } from "./SidebarScene";
import { AreaFocusScene } from "./AreaFocusScene";
import { OnboardingScene, onboardingSceneParams } from "./OnboardingScene";
import { SessionWorkflowScene } from "./SessionWorkflowScene";
import { CoreMoveScene, coreMoveSceneParams } from "./CoreMoveScene";
import { NodeProjectScene, nodeProjectSceneParams } from "./NodeProjectScene";
import { LandingScene, landingSceneParams } from "./LandingScene";
import { MemoryScene, memorySceneParams } from "./MemoryScene";
import { OverviewScene, overviewSceneParams } from "./OverviewScene";
import { UsageScene } from "./UsageScene";
import { ProjectSettingsScene } from "./ProjectSettingsScene";
import { SessionWorkScene } from "./SessionWorkScene";
import { PrCardScene } from "./PrCardScene";
import { PALETTE_HIDE_CLASS } from "../theme";

export function mountGallery(root: Root) {
  // The gallery draws Neutral, the palette every Pen frame it is compared with
  // draws (hide-color-theme D-10), though it shares the page whose first paint is Hide.
  document.documentElement.classList.remove(PALETTE_HIDE_CLASS);
  const params = new URLSearchParams(window.location.search);
  const language = params.get("lang");
  if (language) {
    const selected = requireInterfaceLanguage(language);
    void clientI18n.changeLanguage(selected);
    document.documentElement.lang = selected;
  }
  const scene = params.get("scene");
  if (scene !== null) {
    if (scene === "group-tabs") {
      document.title = "hide · Group tabs scene";
      root.render(<StrictMode><GroupTabsScene params={params} /></StrictMode>);
      return;
    }
    if (scene === "project-settings") {
      root.render(<StrictMode><ProjectSettingsScene params={params} /></StrictMode>);
      return;
    }
    if (scene === "pr-card") {
      root.render(<StrictMode><PrCardScene params={params} /></StrictMode>);
      return;
    }
    if (scene === "session-record") {
      root.render(<StrictMode><SessionWorkScene params={params} /></StrictMode>);
      return;
    }
    if (scene === "usage" || scene === "usage-history") {
      if (scene === "usage-history") {
        params.set("surface", "history-year");
        params.set("history", params.get("history") ?? (params.get("content") === "long" ? "unpriced" : "reference"));
      }
      root.render(<StrictMode><UsageScene params={params} /></StrictMode>);
      return;
    }
    if (scene === "session-panel" || scene === "pane-header") {
      root.render(<StrictMode><SessionWorkflowScene scene={scene} {...sceneParams(params)} /></StrictMode>);
      return;
    }
    if (scene === "area-focus") {
      root.render(<StrictMode><AreaFocusScene theme={params.get("theme") === "light" ? "light" : "dark"} /></StrictMode>);
      return;
    }
    // A scene seeds the app's own stores, so it is one document per scene.
    if (scene === "workspace-servers") {
      root.render(<StrictMode><ServerSessionScene scene={scene} {...sceneParams(params)}/></StrictMode>);
      return;
    }
    if (scene === "right-lane") {
      document.title = "hide · Right lane scene";
      root.render(<StrictMode><RightLaneScene {...rightLaneSceneParams(params)} /></StrictMode>);
      return;
    }
    if (scene === "view-opener-feedback") {
      document.title = "hide · View feedback scene";
      root.render(<StrictMode><ViewFeedbackScene {...viewFeedbackSceneParams(params)} /></StrictMode>);
      return;
    }
    if (scene === "onboarding") {
      document.title = "hide · First run scene";
      root.render(<StrictMode><OnboardingScene {...onboardingSceneParams(params)} /></StrictMode>);
      return;
    }
    if (scene === "search-palette") {
      document.title = "hide · ⌘K scene";
      root.render(<StrictMode><CmdkScene {...cmdkSceneParams(params)} /></StrictMode>);
      return;
    }
    if (scene === "landing") {
      document.title = "hide · Landing shots scene";
      root.render(<StrictMode><LandingScene {...landingSceneParams(params)} /></StrictMode>);
      return;
    }
    if (scene === "memory") {
      document.title = "hide · Memory scene";
      root.render(<StrictMode><MemoryScene {...memorySceneParams(params)} /></StrictMode>);
      return;
    }
    if (scene === "overview") {
      document.title = "hide · Device Overview scene";
      // No StrictMode here: its simulated unmount runs the lane record's
      // leave, which closes the record a production build keeps open.
      root.render(<OverviewScene {...overviewSceneParams(params)} />);
      return;
    }
    if (scene === "core-move" || scene === "core-move-window") {
      document.title = "hide · Core move scene";
      root.render(<StrictMode><CoreMoveScene {...coreMoveSceneParams(scene, params)} /></StrictMode>);
      return;
    }
    if (scene === "node-project") {
      document.title = "hide · Node project scene";
      root.render(<StrictMode><NodeProjectScene {...nodeProjectSceneParams(params)} /></StrictMode>);
      return;
    }
    if (scene !== "projects-sidebar" && scene !== "sidebar-status-axis") throw new Error(`Unknown gallery scene ${scene}`);
    document.title = "hide · Projects sidebar scene";
    root.render(
      <StrictMode>
        <SidebarScene {...sceneParams(params)} axis={scene === "sidebar-status-axis"} />
      </StrictMode>,
    );
    return;
  }
  const frame = params.get("frame");
  const theme = params.get("theme") === "light" ? "light" : "dark";
  if (frame) {
    // A frame's layer takes focus as it opens; without this the gallery page
    // around the frame would scroll to it on every load.
    const focus = HTMLElement.prototype.focus;
    HTMLElement.prototype.focus = function (options?: FocusOptions) {
      focus.call(this, { ...options, preventScroll: true });
    };
    const cut = frame.indexOf("/");
    const section = frame.slice(0, cut) as Section;
    if (!(section in GALLERY)) throw new Error(`Unknown gallery section ${section}`);
    root.render(
      <StrictMode>
        <GalleryFrame section={section} state={frame.slice(cut + 1)} theme={theme} />
      </StrictMode>,
    );
    return;
  }
  document.title = "hide · System gallery";
  root.render(
    <StrictMode>
      <Gallery />
    </StrictMode>,
  );
}
