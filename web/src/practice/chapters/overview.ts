// Flow 1, what a newcomer meets first, on a window with work in every state:
// the three parts, the left list read two ways, what each task group means, a window
// and its two lines, the children and the mission to press, the Explorer,
// search and other computers. One ring and one card a screen; ⏎ once each, or a press where a card asks for one.

import { chapter } from "../chapter";
import { TOUR_WORLD, type TourModel } from "../worlds/tour";

/** The two raised sections above the project list and the rule under them. */
const RAISED = ["[data-raised-group]", "[data-project-list] > [role=separator]"];

/** The keyboard pane's green outline: the ring is the only outline a card wants. */
const OUTLINE = ["[data-pane-focus-outline]"];

export default chapter<TourModel>({
  id: "overview",
  number: 1,
  version: 17,
  world: TOUR_WORLD,
  preview: "zones",
  steps: [
    // The three parts, a ring at a time: the list to choose from, the windows to work in, the rail that opens what to look at.
    { id: "left", target: "[data-sidebar-box]", scene: { lens: "tasks", lane: "closed", focus: null, solo: null } },
    { id: "middle", target: "[data-agent-area]", hides: OUTLINE },
    { id: "right", target: "[data-tool-rail]" },
    // The project view: the switch with its 프로젝트 tab on and the list under it with 내 차례 · 읽을 것 folded away, then one line of it (one session), then the marks on that line.
    { id: "projects", target: "[data-sidebar-lens-option], [data-project-list]", hides: RAISED, scene: { lens: "projects" } },
    { id: "session", target: '[data-sidebar-box] [data-pane="a2"]', hides: RAISED },
    { id: "pr", target: '[data-pane="a2"] [data-pr-mark]', hides: RAISED },
    { id: "mission", target: '[data-pane="a2"] [data-mission-chip]', hides: RAISED },
    // The task view: the switch with its 태스크 tab on and 내 차례 under it, then what finished quietly and what is working.
    { id: "needs", target: '[data-sidebar-lens-option], [data-task-group="needs_you"] > *', scene: { lens: "tasks" } },
    { id: "read", target: '[data-task-group="done"] > *' },
    // Two kinds of Working: a session working by itself, and one waiting on its children (its second line says so).
    { id: "work", target: '[data-task-group="working"] > *' },
    // A window (the Conductor's), then its two lines: what stands on the first, what the second says of the session's mission part and children.
    { id: "pane", target: '[data-pane-view="a2"]', hides: OUTLINE },
    { id: "line1", target: '[data-pane-view="a2"] [data-pane-move-row]', hides: OUTLINE, scene: { solo: "a2" } },
    { id: "line2", target: '[data-pane-view="a2"] [data-pane-header-band]', hides: OUTLINE },
    // Press the children chip, then the list it opened: the card presses the chip again when it is left, so the list is open exactly while the card talks about it.
    { id: "tree", target: '[data-pane-tree="a2"]', hides: OUTLINE, press: true },
    { id: "kids", target: '[data-pane-tree="a2"], [data-agent-tree="a2"]', hides: OUTLINE, opens: '[data-pane-tree="a2"]' },
    // Press the mission chip: the pane's own press opens that mission's map in the 보기 slot beside the pane.
    { id: "missionChip", target: '[data-pane-view="a2"] [data-pane-mission-chip]', hides: OUTLINE, press: true },
    { id: "conduct", target: "[data-lane-column]", hides: OUTLINE, keepClear: ["[data-agent-area]"], scene: { lane: "mission" } },
    // The Explorer: its button first, then what it shows of the agent's work (changed files marked, a result opened beside the tree).
    { id: "explorerButton", target: '[data-lane-tool="explorer"]', press: true, commands: ["toggle_lane_explorer"], scene: { lane: "closed" } },
    { id: "explorer", target: "[data-lane-column]", scene: { lane: "explorer", focus: "q1", solo: "q1" } },
    { id: "find", target: "[data-sidebar-search]", commands: ["search"], scene: { lane: "closed", solo: null } },
    { id: "devices", target: "[data-device-rail]" },
  ],
});
