// Flow 1, what a newcomer meets first, on a window with work in every state:
// the three parts, the left list read two ways, what each task group means, more
// agents, the Explorer, what a mission is, search and other computers. One card a screen, ⏎ once each.

import { chapter } from "../chapter";
import { TOUR_WORLD, type TourModel } from "../worlds/tour";

export default chapter<TourModel>({
  id: "overview",
  number: 1,
  version: 14,
  world: TOUR_WORLD,
  preview: "zones",
  steps: [
    { id: "parts", zones: true, scene: { lens: "tasks", lane: "closed", focus: null, solo: null } },
    // The project view alone: 내 차례 · 읽을 것 folded away so the projects fill the ring, each mark of a row named by a tag. One agent in the middle leaves the space beside the list empty for the tags.
    {
      id: "projects", target: "[data-project-list]", hides: ["[data-raised-group]", "[data-project-list] > [role=separator]"], scene: { lens: "projects", solo: "a1" },
      callouts: [
        { target: '[data-checkout-row="hide:main"] [data-checkout-age]', say: "branch" },
        { target: '[data-other-checkouts="hide"]', say: "others" },
        { target: '[data-pane="a1"] [data-pr-counts]', say: "childPrs" },
        { target: '[data-pane="a2"] [data-mission-chip]', say: "mission" },
        { target: '[data-pane="a2"] [data-pr-mark]', say: "pr" },
      ],
    },
    // The task view, one group a card: what waits for the operator, what finished quietly, what is working.
    { id: "needs", target: '[data-task-group="needs_you"] > *', scene: { lens: "tasks" }, callouts: [{ target: '[data-task-row="a3"] [data-line-part="text"]', say: "ask" }, { target: '[data-task-row="q1"] [data-line-part]', say: "look" }] },
    { id: "read", target: '[data-task-group="done"] > *', callouts: [{ target: '[data-task-row="q2"] [data-line-part]', say: "done" }] },
    // Two kinds of Working: a session working by itself, and one waiting on its children (its second line says so).
    { id: "work", target: '[data-task-group="working"] > *', callouts: [{ target: '[data-task-row="a1"] [data-line-part]', say: "waiting" }] },
    // The head of an agent's window: its first line says who and how it stands, its second what it leads or was asked.
    { id: "head", target: ['[data-pane-view="a2"] [data-pane-move-row]', '[data-pane-view="a2"] [data-pane-header-band]'], scene: { focus: "a2", solo: "a2" },
      callouts: [
        { target: '[data-pane-view="a2"] [data-pane-status-mark]', say: "state" },
        { target: '[data-pane-view="a2"] [data-agent-title]', say: "title" },
        { target: '[data-pane-view="a2"] [data-pr-mark]', say: "pr" },
        { target: '[data-pane-view="a2"] [data-pane-mission-chip]', say: "mission" },
        { target: '[data-pane-view="a2"] [data-pane-tree]', say: "tree" },
        { target: '[data-pane-view="a2"] [data-pane-own-progress]', say: "line" },
      ],
    },
    // A lead's children are listed from the chip on its pane head's second line: the card opens that list and rings the chip and the list.
    { id: "agents", target: ['[data-pane-tree="a1"]', '[data-agent-tree="a1"]'], opens: '[data-pane-tree="a1"]', scene: { focus: "a1", solo: null } },
    // The agent's work in the Explorer: changed files marked, a result opened beside the tree.
    { id: "explorer", target: "[data-lane-column]", commands: ["toggle_lane_explorer"], scene: { lane: "explorer", focus: "q1", solo: "q1" } },
    // A mission's map in the 보기 slot, beside the Conductor's pane, whose first line is the sentence the card quotes.
    { id: "conduct", target: "[data-lane-column]", keepClear: ["[data-agent-area]"], scene: { lane: "mission", solo: "a2" } },
    { id: "find", target: "[data-sidebar-search]", commands: ["search"], scene: { lane: "closed", solo: null } },
    { id: "devices", target: "[data-device-rail]" },
  ],
});
