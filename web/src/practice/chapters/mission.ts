// Chapter 0, where the first run and the sidebar's invitation land: handing a
// Conductor a job of several pull requests as a mission, seeing the work it
// split on the mission's map in the 보기 slot, finding the one decision it asks
// there, on the 태스크 list and on its pane's head, answering it on the map,
// the Overview named in passing, and what was decided kept in the Project's
// Memory.

import { chapter } from "../chapter";
import { CONDUCTOR, MISSION_WORLD, missionInSlot, showsMemory, showsTasks, type MissionModel } from "../worlds/mission";

const PANE = `[data-pane-view="${CONDUCTOR}"]`;
const CHIP = `${PANE} [data-pane-mission-chip]`;
/** The banner's recommended choice, or, while the map is not open, the chip that opens it. */
const ANSWER = `[data-mission-recommended], body:not(:has([data-mission-recommended])) ${CHIP}`;

export default chapter<MissionModel>({
  id: "mission",
  number: 0,
  version: 4,
  world: MISSION_WORLD,
  quest: true,
  steps: [
    { id: "intro", target: PANE, side: "inside" },
    // Any words make the mission: the Conductor plans it, starts its workers, and asks one decision.
    { id: "ask", quest: true, target: PANE, side: "inside", gesture: { type: { pane: CONDUCTOR, words: "practice.mission.world.example" } }, done: (model) => model.stage >= 1 },
    // The chip opens the map in the 보기 slot, where the plan's split is seen; a first plan also opens the chip's popover once.
    { id: "split", quest: true, target: CHIP, side: "right", gesture: { click: true }, done: missionInSlot },
    { id: "nodes", target: "[data-mission] [data-mission-node]", side: "left" },
    { id: "question", target: '[data-mission-banner="decision"]', side: "left" },
    // The same ask on the sidebar's 태스크 list and on the Conductor pane's head.
    { id: "tasks", quest: true, target: "[data-sidebar-lens]", side: "right", command: "toggle_sidebar_lens", done: showsTasks },
    { id: "rows", target: `[data-task-row="${CONDUCTOR}"], ${PANE} [data-pane-header-band]`, side: "right" },
    // The banner takes the answer: a choice, or `직접 답하기…`; words said to the Conductor answer it too.
    { id: "answer", quest: true, target: ANSWER, side: "left", gesture: { click: true }, done: (model) => model.stage >= 2 },
    { id: "map", target: "[data-mission]", side: "left" },
    // Named in passing: the Overview is a place to look, not a step to take.
    { id: "overview", target: "[data-sidebar-overview]", side: "right" },
    // ⇧⌘L opens Hide AI on its Memory tab; the rail icon would open it on 제안.
    { id: "memory", quest: true, target: '[data-lane-tool="memory"]', side: "left", command: "open_project_memory", done: showsMemory },
    { id: "end", target: "[data-hide-ai-tool]", side: "left" },
  ],
});
