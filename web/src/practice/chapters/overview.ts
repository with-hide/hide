// Flow 1, what a newcomer meets first, on a window with work in every state:
// the three parts, the left list read two ways, what each task group means, more
// agents, the Explorer, what a mission is, search and other computers. One card a screen, ⏎ once each.

import { chapter } from "../chapter";
import { TOUR_WORLD, type TourModel } from "../worlds/tour";

/** The two raised sections above the project list and the rule under them. */
const RAISED = ["[data-raised-group]", "[data-project-list] > [role=separator]"];

export default chapter<TourModel>({
  id: "overview",
  number: 1,
  version: 15,
  world: TOUR_WORLD,
  preview: "zones",
  steps: [
    { id: "parts", zones: true, scene: { lens: "tasks", lane: "closed", focus: null, solo: null } },
    // The project view, a ring at a time: the list with 내 차례 · 읽을 것 folded away, then the PR mark of one row, then its mission chip. One agent is shown in the middle so the list is short.
    { id: "projects", target: "[data-project-list]", hides: RAISED, scene: { lens: "projects", solo: "a1" } },
    { id: "pr", target: '[data-pane="a2"] [data-pr-mark]', hides: RAISED },
    { id: "mission", target: '[data-pane="a2"] [data-mission-chip]', hides: RAISED },
    // The task view, one group a card: what waits for the operator, what finished quietly, what is working.
    { id: "needs", target: '[data-task-group="needs_you"] > *', scene: { lens: "tasks" } },
    { id: "read", target: '[data-task-group="done"] > *' },
    // Two kinds of Working: a session working by itself, and one waiting on its children (its second line says so).
    { id: "work", target: '[data-task-group="working"] > *' },
    // The first line of an agent's window: its state, task, PR and mission progress, as one line.
    { id: "head", target: '[data-pane-view="a2"] [data-pane-move-row]', scene: { focus: "a2", solo: "a2" } },
    // A lead's children are listed from the chip on its window head's second line: the card opens that list and rings the chip and the list as one box.
    { id: "agents", target: '[data-pane-tree="a1"], [data-agent-tree="a1"]', opens: '[data-pane-tree="a1"]', scene: { focus: "a1", solo: null } },
    // The agent's work in the Explorer: changed files marked, a result opened beside the tree.
    { id: "explorer", target: "[data-lane-column]", commands: ["toggle_lane_explorer"], scene: { lane: "explorer", focus: "q1", solo: "q1" } },
    // A mission's map in the 보기 slot, beside the Conductor's pane, whose first line is the sentence the card quotes.
    { id: "conduct", target: "[data-lane-column]", keepClear: ["[data-agent-area]"], scene: { lane: "mission", solo: "a2" } },
    { id: "find", target: "[data-sidebar-search]", commands: ["search"], scene: { lane: "closed", solo: null } },
    { id: "devices", target: "[data-device-rail]" },
  ],
});
