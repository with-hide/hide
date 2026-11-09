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
  version: 9,
  world: TOUR_WORLD,
  preview: "zones",
  steps: [
    { id: "parts", zones: true },
    { id: "left", target: "[data-sidebar-box] [data-sidebar]" },
    // Needs You holds only what waits for the operator; finished work goes quietly to Unread.
    { id: "turn", target: '[data-task-group="needs_you"], [data-task-group="done"]' },
    // Two kinds of Working: a session working by itself, and one waiting on its children (its second line says so).
    { id: "work", target: '[data-task-group="working"]' },
    // Splitting happens where the agents' terminals are.
    { id: "spawn", target: "[data-agent-area]", command: "split_right" },
    // The row marked To look at: pressing it shows what the agent opened on the right.
    { id: "look", target: '[data-task-row="q1"]' },
    { id: "find", target: "[data-sidebar-search]", command: "search" },
    // The right rail: what its icons open, and the pin that keeps a tool beside the agents.
    { id: "tools", target: "[data-tool-rail]" },
    // The device rail: another computer is a tile that switches the screen to it.
    { id: "devices", target: "[data-device-rail]" },
    // What a mission is, over the agents it would run.
    { id: "aboutMission", target: "[data-agent-area]" },
    { id: "mission", mission: true },
  ],
});
