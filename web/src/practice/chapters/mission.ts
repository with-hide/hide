// Chapter 0, where the first run and the sidebar's invitation land: one job
// carried from the learner's request to the report, as a mission, at the pace
// an agent works: moving sign-in to email links, six parts two workers at a
// time. The learner asks a Conductor for a plan with `/hide-mission`, opens it
// from the pane's mission chip and approves it in the terminal; the pane head's second row lists the child agents it starts, one opens in its tab and its head leads back, they run, and when the Conductor has a question it
// lands in the sidebar's 내 차례, where the learner opens it and answers on the
// map. The Overview shows the team at work until it is done, and what the work
// left is the Conductor's report, a new memory in the Project's Memory, and the
// changed files in the Explorer, one of them opened beside it.

import { chapter } from "../chapter";
import { CONDUCTOR, MISSION_WORLD, SIGN_IN_FILE, focusedOn, missionInSlot, openedFile, showsExplorer, showsMemory, type MissionModel } from "../worlds/mission";

const PANE = `[data-pane-view="${CONDUCTOR}"]`;
const CHIP = `${PANE} [data-pane-mission-chip]`;
/** The banner's recommended choice, or, while the map is not open, the chip that opens it. */
const ANSWER = `[data-mission-recommended], body:not(:has([data-mission-recommended])) ${CHIP}`;
const GRAPH = "[data-overview-graph]";
/** The sign-in screen's worker, the one that runs longest: the list holds it once the survey is done. */
const SCREEN = "ui";

export default chapter<MissionModel>({
  id: "mission",
  number: 0,
  version: 10,
  world: MISSION_WORLD,
  quest: true,
  steps: [
    // The situation, handed over with `/hide-mission` from the first step: the Conductor plans for a few seconds before the plan stands.
    { id: "intro", quest: true, target: PANE, side: "center", gesture: { type: { pane: CONDUCTOR, words: "practice.mission.world.example" } }, done: (model) => model.stage >= 1 },
    { id: "thinking", target: PANE, side: "center", wait: true, done: (model) => model.stage >= 2 },
    // The chip opens the whole plan in the 보기 slot; a first plan also opens the chip's popover once.
    { id: "chip", quest: true, target: CHIP, side: "right", gesture: { click: true }, done: missionInSlot },
    { id: "plan", target: "[data-mission]", side: "left" },
    // The go is said to the Conductor in the terminal, because Hide has no button that starts a mission.
    { id: "approve", quest: true, target: PANE, side: "center", gesture: { type: { pane: CONDUCTOR, words: "practice.mission.world.approve" } }, done: (model) => model.stage >= 3 },
    // The pane head's second row is the Conductor's child agents: it opens the list, a child opens its tab, and the child's head leads back.
    { id: "children", quest: true, target: `[data-pane-tree="${CONDUCTOR}"]`, side: "right", gesture: { click: true }, shown: `[data-agent-child="${SCREEN}"]` },
    { id: "child", quest: true, target: `[data-agent-child="${SCREEN}"]`, side: "right", gesture: { click: true }, done: (model) => focusedOn(model) === SCREEN },
    { id: "back", quest: true, target: `[data-pane-return="${CONDUCTOR}"]`, side: "bottom", gesture: { click: true }, done: (model) => focusedOn(model) === CONDUCTOR },
    // The workers run until the Conductor has a question.
    { id: "running", target: "[data-mission]", side: "left", wait: true, done: (model) => model.stage >= 5 },
    // The question lands in 내 차례; the row opens the Conductor.
    { id: "needsyou", quest: true, target: `[data-task-row="${CONDUCTOR}"]`, side: "right", gesture: { click: true }, done: (model) => model.looked || model.stage >= 6 },
    // The map's banner takes the answer: a choice, or `직접 답하기…`; words said to the Conductor answer it too.
    { id: "answer", quest: true, target: ANSWER, side: "left", gesture: { click: true }, done: (model) => model.stage >= 6 },
    { id: "overview", quest: true, target: "[data-sidebar-overview]", side: "right", gesture: { click: true }, shown: GRAPH },
    // The team at work on the map until the last worker is done.
    // The bubble stands under the graph, so its nodes stay in sight.
    { id: "graph", target: GRAPH, side: "bottom", wait: true, done: (model) => model.stage >= 9 },
    // Done: the Overview put away by its own shortcut, or Esc, for what the work left.
    { id: "finished", target: GRAPH, side: "bottom", command: "overview", shown: `body:not(:has(${GRAPH}))` },
    { id: "report", target: "[data-mission]", side: "left" },
    // ⇧⌘L opens Hide AI on its Memory tab; the rail icon would open it on 제안.
    { id: "memory", quest: true, target: '[data-lane-tool="memory"]', side: "left", command: "open_project_memory", done: showsMemory },
    // The files the parts changed, in the Explorer, and one of them opened beside it.
    { id: "explorer", quest: true, target: '[data-lane-tool="explorer"]', side: "left", command: "toggle_lane_explorer", done: showsExplorer },
    { id: "file", quest: true, target: `[data-explorer-row="${SIGN_IN_FILE}"]`, side: "left", gesture: { click: true }, done: (model) => openedFile(model) === SIGN_IN_FILE },
    // The bubble keeps to the terminal, so the opened file stays readable.
    { id: "end", target: PANE, side: "inside" },
  ],
});
