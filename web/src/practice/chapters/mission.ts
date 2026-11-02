// Chapter 0, where the first run and the sidebar's invitation land: one job
// carried from the learner's request to the report, as a mission. The learner
// hands the Conductor a situation, sees the plan on the mission's map and
// approves it, finds the one decision it asks there, on the 태스크 list and on
// its pane's head, answers it, looks in on the worker the Conductor started
// and comes back to the report, sees the team on the Overview's map, and finds
// what was decided kept in the Project's Memory.

import { chapter } from "../chapter";
import { CONDUCTOR, MISSION_WORLD, WORKER, focusedOn, missionInSlot, showsMemory, showsTasks, type MissionModel } from "../worlds/mission";

const PANE = `[data-pane-view="${CONDUCTOR}"]`;
const CHIP = `${PANE} [data-pane-mission-chip]`;
/** The banner's recommended choice, or, while the map is not open, the chip that opens it. */
const ANSWER = `[data-mission-recommended], body:not(:has([data-mission-recommended])) ${CHIP}`;

export default chapter<MissionModel>({
  id: "mission",
  number: 0,
  version: 5,
  world: MISSION_WORLD,
  quest: true,
  steps: [
    { id: "intro", target: PANE, side: "inside" },
    // The situation, handed over with `/hide-mission`: the Conductor only plans it, and asks before it starts anything.
    { id: "ask", quest: true, target: PANE, side: "inside", gesture: { type: { pane: CONDUCTOR, words: "practice.mission.world.example" } }, done: (model) => model.stage >= 1 },
    // The chip opens the plan on the map in the 보기 slot; a first plan also opens the chip's popover once.
    { id: "plan", quest: true, target: CHIP, side: "right", gesture: { click: true }, done: missionInSlot },
    { id: "nodes", target: "[data-mission] [data-mission-node]", side: "left" },
    // The go is said to the Conductor in its own words: it starts the worker, and the map turns to 진행 중.
    { id: "approve", quest: true, target: PANE, side: "inside", gesture: { type: { pane: CONDUCTOR, words: "practice.mission.world.approve" } }, done: (model) => model.stage >= 2 },
    { id: "question", target: '[data-mission-banner="decision"]', side: "left" },
    // The same ask on the sidebar's 태스크 list and on the Conductor pane's head.
    { id: "tasks", quest: true, target: "[data-sidebar-lens]", side: "right", command: "toggle_sidebar_lens", done: showsTasks },
    { id: "rows", target: `[data-task-row="${CONDUCTOR}"], ${PANE} [data-pane-header-band]`, side: "right" },
    // The banner takes the answer: a choice, or `직접 답하기…`; words said to the Conductor answer it too.
    { id: "answer", quest: true, target: ANSWER, side: "left", gesture: { click: true }, done: (model) => model.stage >= 3 },
    { id: "map", target: "[data-mission]", side: "left" },
    // The worker the Conductor started, from the pane head's tree, and the way back; coming back, the work is done.
    { id: "children", quest: true, target: `[data-pane-tree="${CONDUCTOR}"]`, side: "right", gesture: { click: true }, shown: `[data-agent-tree="${CONDUCTOR}"]` },
    { id: "child", quest: true, target: `[data-agent-child="${WORKER}"]`, side: "right", gesture: { click: true }, done: (model) => focusedOn(model) === WORKER },
    { id: "back", quest: true, target: `[data-pane-return="${CONDUCTOR}"]`, side: "bottom", gesture: { click: true }, done: (model) => model.stage >= 4 },
    { id: "report", target: "[data-mission]", side: "left" },
    // 개요에서 보기, first in the pane's menu (⋯ or right-click), opens the Overview with the Conductor chosen.
    {
      id: "overview", quest: true, target: `${PANE} header, [role="menu"]`, side: "bottom",
      gesture: { menu: { at: `${PANE} header`, path: ['[data-menu-item="open_in_overview"]'] } }, shown: "[data-overview-graph]",
    },
    { id: "graph", target: "[data-overview-graph]", side: "inside" },
    // The Overview's own shortcut, or Esc, puts it away; what the work left is beside the terminal.
    { id: "close", target: "[data-overview-graph]", side: "inside", command: "overview", shown: "body:not(:has([data-overview-graph]))" },
    // ⇧⌘L opens Hide AI on its Memory tab; the rail icon would open it on 제안.
    { id: "memory", quest: true, target: '[data-lane-tool="memory"]', side: "left", command: "open_project_memory", done: showsMemory },
    { id: "end", target: "[data-hide-ai-tool]", side: "left" },
  ],
});
