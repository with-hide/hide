// Flow 1: the three parts of the screen, pointed at on a window with work in them, and ⌘K to find.

import { chapter } from "../chapter";
import { TOUR_WORLD, type TourModel } from "../worlds/tour";

export default chapter<TourModel>({
  id: "overview",
  number: 1,
  version: 3,
  world: TOUR_WORLD,
  quest: true,
  preview: "zones",
  steps: [
    { id: "parts", zones: true, side: "center" },
    { id: "left", target: "[data-sidebar-box] [data-sidebar]", side: "right" },
    { id: "lenses", quest: true, target: "[data-sidebar-lens]", side: "right", command: "toggle_sidebar_lens", done: (model) => model.seen.length === 3 },
    { id: "search", quest: true, target: "[data-sidebar-search]", side: "right", command: "search", gesture: { click: true }, done: (model) => model.searched },
    // While the palette is up it holds the keyboard; the guide comes back to the next step when Esc or ⏎ closes it.
    { id: "center", target: "[data-agent-area]", side: "right" },
    { id: "lane", target: "[data-lane-slot]", side: "left" },
    { id: "tools", target: "[data-lane-tool]", side: "left" },
    { id: "devices", target: "[data-device-rail]", side: "right" },
    { id: "end", zones: true, side: "center" },
  ],
});
