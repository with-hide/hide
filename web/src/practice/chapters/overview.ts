// Chapter 0: the three parts of the screen, pointed at on a window with work in them.

import { chapter } from "../chapter";
import { TOUR_WORLD, type TourModel } from "../worlds/tour";

export default chapter<TourModel>({
  id: "overview",
  number: 0,
  version: 1,
  world: TOUR_WORLD,
  quest: false,
  steps: [
    { id: "parts", zones: true, side: "center" },
    { id: "left", target: "[data-sidebar-box] [data-sidebar]", side: "right" },
    { id: "lenses", target: "[data-sidebar-lens]", side: "right", command: "toggle_sidebar_lens", done: (model) => model.seen.length === 3 },
    { id: "search", target: "[data-sidebar-search]", side: "right" },
    { id: "center", target: "[data-agent-area]", side: "right" },
    { id: "lane", target: "[data-lane-slot]", side: "left" },
    { id: "todo", target: "[data-project-todo]", side: "left" },
    { id: "tools", target: "[data-lane-tool]", side: "left" },
    { id: "devices", target: "[data-device-rail]", side: "right" },
    { id: "end", zones: true, side: "center" },
  ],
});
