// Chapter 0, where the first run and the sidebar's invitation land: one job
// carried from the learner's request to the report, as a mission, at the pace
// an agent works. The learner asks a Conductor for a plan with
// `/hide-mission`, opens it from the pane's mission chip and approves it in
// the terminal; the workers run, and when the Conductor has a question it
// lands in the sidebar's 내 차례, where the learner opens it and answers on the
// map. The Overview shows the team at work until it is done, and what the work
// left is the Conductor's report and a new memory in the Project's Memory.

import { chapter } from "../chapter";
import { CONDUCTOR, MISSION_WORLD, missionInSlot, showsMemory, type MissionModel } from "../worlds/mission";

const PANE = `[data-pane-view="${CONDUCTOR}"]`;
const CHIP = `${PANE} [data-pane-mission-chip]`;
/** The banner's recommended choice, or, while the map is not open, the chip that opens it. */
const ANSWER = `[data-mission-recommended], body:not(:has([data-mission-recommended])) ${CHIP}`;
const GRAPH = "[data-overview-graph]";

export default chapter<MissionModel>({
  id: "mission",
  number: 0,
  version: 6,
  world: MISSION_WORLD,
  quest: true,
  steps: [
    // The bubble keeps low while the Conductor writes, so its lines stay readable.
    { id: "intro", target: PANE, side: "inside" },
    // The situation, handed over with `/hide-mission`: the Conductor plans for a few seconds before the plan stands.
    { id: "ask", quest: true, target: PANE, side: "center", gesture: { type: { pane: CONDUCTOR, words: "practice.mission.world.example" } }, done: (model) => model.stage >= 1 },
    { id: "thinking", target: PANE, side: "center", wait: true, done: (model) => model.stage >= 2 },
    // The chip opens the whole plan in the 보기 slot; a first plan also opens the chip's popover once.
    { id: "chip", quest: true, target: CHIP, side: "right", gesture: { click: true }, done: missionInSlot },
    { id: "plan", target: "[data-mission]", side: "left" },
    // The go is said to the Conductor in the terminal, because Hide has no button that starts a mission.
    { id: "approve", quest: true, target: PANE, side: "center", gesture: { type: { pane: CONDUCTOR, words: "practice.mission.world.approve" } }, done: (model) => model.stage >= 3 },
    // The workers run until the Conductor has a question.
    { id: "running", target: "[data-mission]", side: "left", wait: true, done: (model) => model.stage >= 4 },
    // The question lands in 내 차례; the row opens the Conductor.
    { id: "needsyou", quest: true, target: `[data-task-row="${CONDUCTOR}"]`, side: "right", gesture: { click: true }, done: (model) => model.looked || model.stage >= 5 },
    // The map's banner takes the answer: a choice, or `직접 답하기…`; words said to the Conductor answer it too.
    { id: "answer", quest: true, target: ANSWER, side: "left", gesture: { click: true }, done: (model) => model.stage >= 5 },
    { id: "overview", quest: true, target: "[data-sidebar-overview]", side: "right", gesture: { click: true }, shown: GRAPH },
    // The team at work on the map until the last worker is done.
    { id: "graph", target: GRAPH, side: "inside", wait: true, done: (model) => model.stage >= 6 },
    // Done: the Overview put away by its own shortcut, or Esc, for what the work left.
    { id: "finished", target: GRAPH, side: "inside", command: "overview", shown: `body:not(:has(${GRAPH}))` },
    { id: "report", target: "[data-mission]", side: "left" },
    // ⇧⌘L opens Hide AI on its Memory tab; the rail icon would open it on 제안.
    { id: "memory", quest: true, target: '[data-lane-tool="memory"]', side: "left", command: "open_project_memory", done: showsMemory },
    { id: "end", target: "[data-hide-ai-tool]", side: "left" },
  ],
});
