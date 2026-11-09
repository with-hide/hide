// Flow 1, what a newcomer meets first, on a window with work in every state:
// the three parts, the left list read two ways, what each state means, more
// agents, what an agent opened to look at, search, the right rail, other
// computers and what a mission is. One card a screen, ⏎ once each.

import { chapter } from "../chapter";
import { TOUR_WORLD, type TourModel } from "../worlds/tour";

export default chapter<TourModel>({
  id: "overview",
  number: 1,
  version: 12,
  world: TOUR_WORLD,
  preview: "zones",
  steps: [
    { id: "parts", zones: true, scene: { lens: "tasks", lane: "closed", focus: null, solo: null } },
    // The left list read two ways; each card turns the list to its view itself.
    { id: "projects", target: "[data-raised-list], [data-project-list]", scene: { lens: "projects" } },
    { id: "rows", target: '[data-checkout-row="hide:electron"]' },
    // Needs You holds only what waits for the operator; finished work goes quietly to Unread.
    { id: "tasks", target: "[data-task-list]", scene: { lens: "tasks" } },
    // Two kinds of Working: a session working by itself, and one waiting on its children (its second line says so).
    { id: "work", target: '[data-task-group="working"] > *' },
    // A lead's children are listed from the chip on its pane head's second line: the card opens that list and rings the chip and the list.
    { id: "agents", target: ['[data-pane-tree="a1"]', '[data-agent-tree="a1"]'], opens: '[data-pane-tree="a1"]', scene: { focus: "a1" } },
    // One move: the 볼 것 row brings the agent that opened a file forward in the middle and its file onto the right.
    { id: "files", target: ['[data-task-row="q1"]', "[data-lane-column]"], scene: { lane: "documents", focus: "q1", solo: "q1" } },
    // The session's own files: the Explorer's folder tree beside the file, in the same lane.
    { id: "explorer", target: "[data-lane-column]", commands: ["toggle_lane_explorer"], scene: { lane: "explorer" } },
    // A mission's map in the 보기 slot, beside the Conductor's pane, whose first line is the sentence the card quotes.
    { id: "conduct", target: "[data-lane-column]", keepClear: ["[data-agent-area]"], scene: { lane: "mission", solo: "a2" } },
    { id: "find", target: "[data-sidebar-search]", commands: ["search"], scene: { lane: "closed", solo: null } },
    { id: "devices", target: "[data-device-rail]" },
  ],
});
