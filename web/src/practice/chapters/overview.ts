// Flow 1, what a newcomer meets first, on a window with work in every state:
// the three parts, the left list read two ways, what each state means, more
// agents, what an agent opened to look at, search, the right rail, other
// computers, what a mission is, and a button that hands over
// to a real mission. One card a screen, ⏎ once each.

import { chapter } from "../chapter";
import { TOUR_WORLD, type TourModel } from "../worlds/tour";

export default chapter<TourModel>({
  id: "overview",
  number: 1,
  version: 11,
  world: TOUR_WORLD,
  preview: "zones",
  steps: [
    { id: "parts", zones: true, scene: { lens: "tasks", lane: "closed", focus: null, solo: null } },
    // The left list read two ways; each card turns the list to its view itself.
    { id: "projects", target: "[data-project-list]", scene: { lens: "projects" } },
    { id: "rows", target: '[data-checkout-row="hide:electron"]' },
    // Needs You holds only what waits for the operator; finished work goes quietly to Unread.
    { id: "tasks", target: "[data-task-list]", scene: { lens: "tasks" } },
    // Two kinds of Working: a session working by itself, and one waiting on its children (its second line says so).
    { id: "work", target: '[data-task-group="working"] > *' },
    // Four panes in one tab; a lead's children show on its pane head's second line.
    { id: "agents", target: '[data-pane-header-band]:has([data-pane-tree="a1"])', commands: ["split_right", "split_down"], scene: { focus: "a1" } },
    // An agent opened a file for the operator; it shows in the lane's Documents.
    { id: "files", target: "[data-lane-slot]", scene: { lane: "documents", focus: null, solo: "a1" } },
    // A mission's map in the 보기 slot, beside the Conductor's pane.
    { id: "conduct", target: "[data-lane-slot]", scene: { lane: "mission", solo: "a2" } },
    { id: "find", target: "[data-sidebar-search]", commands: ["search"], scene: { lane: "closed", solo: null } },
    { id: "devices", target: "[data-device-rail]" },
    { id: "mission", mission: true },
  ],
});
