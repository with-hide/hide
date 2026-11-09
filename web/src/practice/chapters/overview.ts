// Flow 1, what a newcomer meets first, on a window with work in every state:
// the three parts, the left list read two ways, what each state means, more
// agents, what an agent opened to look at, search, and a button that hands over
// to a real mission. One card a screen, ⏎ once each.

import { chapter } from "../chapter";
import { TOUR_WORLD, type TourModel } from "../worlds/tour";

export default chapter<TourModel>({
  id: "overview",
  number: 1,
  version: 7,
  world: TOUR_WORLD,
  preview: "zones",
  steps: [
    { id: "parts", zones: true },
    { id: "left", target: "[data-sidebar-box] [data-sidebar]" },
    // Needs You holds only what waits for the operator; finished work goes quietly to Unread, and a working row that waits says so on its second line.
    { id: "states", target: "[data-task-list]" },
    // The lead whose children list under it, and the split that opens another agent beside one.
    { id: "spawn", target: '[data-task-row="a1"]', command: "split_right" },
    // The row marked To look at: pressing it shows what the agent opened on the right.
    { id: "look", target: '[data-task-row="q1"]' },
    { id: "find", target: "[data-sidebar-search]", command: "search" },
    { id: "mission", mission: true },
  ],
});
