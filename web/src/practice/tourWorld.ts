// Chapter 0's world: the introduction site's hero window (a lead, the agent
// asking to merge, and the Project tool beside them), which shows all three
// parts of the screen with work in them. The only thing it answers is the
// sidebar's list, so the learner can turn it through its three lenses.

import { HERO_PANES, HERO_TERMINALS, landingUsage, landingWorkspace, RULE } from "../gallery/landingSceneData";
import { nextSidebarLens, type SidebarLens } from "../snapshot";
import type { PracticeWorld } from "./world";

export type TourModel = { lens: SidebarLens; seen: SidebarLens[] };

export const TOUR_WORLD: PracticeWorld<TourModel> = {
  initial: () => ({ lens: "projects", seen: ["projects"] }),
  apply(model, event) {
    if (event.kind !== "sidebar_lens_set") return null;
    const lens = (event.payload.lens as SidebarLens | undefined) ?? nextSidebarLens(model.lens);
    return { lens, seen: model.seen.includes(lens) ? model.seen : [...model.seen, lens] };
  },
  snapshot(model, nowMs) {
    const { rest, agents } = landingWorkspace(nowMs, 320, [], model.lens);
    return { rest, agents, focused: null, usage: landingUsage(nowMs) };
  },
  panes: () => [...HERO_PANES],
  screen: (_model, paneId, cols) => HERO_TERMINALS[paneId as (typeof HERO_PANES)[number]].replaceAll(RULE, "─".repeat(cols)),
};
