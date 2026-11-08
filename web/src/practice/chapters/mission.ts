// The mission flow (flow 4), where the first run and the sidebar's invitation land: one job
// carried from the learner's request to the report, as a mission, at the pace
// an agent works: moving sign-in to email links, six parts two workers at a
// time. The learner asks a Conductor for a plan with `/hide-mission`, opens it
// from the pane's mission chip and approves it in the terminal; the pane head's
// second row lists the child agents the Conductor starts; when it has a question
// it lands in the sidebar's 내 차례, where the learner opens it and answers on the
// map. What the work left is the Conductor's report and a new memory in the
// Project's Memory.

import { chapter } from "../chapter";
import { CONDUCTOR, DECISION_CARD, MISSION_WORLD, missionInSlot, showsMemory, type MissionModel } from "../worlds/mission";

const PANE = `[data-pane-view="${CONDUCTOR}"]`;
const CHIP = `${PANE} [data-pane-mission-chip]`;
/** The banner's recommended choice, or, while the map is not open, the chip that opens it. */
const ANSWER = `[data-mission-recommended], body:not(:has([data-mission-recommended])) ${CHIP}`;
/** The sign-in screen's worker, the one that runs longest: the list holds it once the survey is done. */
const SCREEN = "ui";

export default chapter<MissionModel>({
  id: "mission",
  number: 4,
  next: "overview",
  version: 14,
  world: MISSION_WORLD,
  quest: true,
  preview: "mission",
  steps: [
    // The situation, handed over with `/hide-mission` from the first step: the Conductor plans for a few seconds before the plan stands.
    // The screen is seen alone a moment first, since it is the first a newcomer meets.
    { id: "intro", quest: true, target: PANE, side: "center", scene: 2400, gesture: { type: { pane: CONDUCTOR, words: "practice.mission.world.example" } }, done: (model) => model.stage >= 1 },
    { id: "thinking", target: PANE, side: "center", wait: true, done: (model) => model.stage >= 2 },
    // The chip opens the whole plan in the 보기 slot; a first plan also opens the chip's popover once.
    { id: "chip", quest: true, target: CHIP, side: "right", gesture: { click: true }, done: missionInSlot },
    { id: "plan", target: "[data-mission]", side: "left" },
    // The go is said to the Conductor in the terminal, because Hide has no button that starts a mission.
    { id: "approve", quest: true, target: PANE, side: "center", gesture: { type: { pane: CONDUCTOR, words: "practice.mission.world.approve" } }, done: (model) => model.stage >= 3 },
    // The pane head's second row is the Conductor's child agents: it opens the list of the workers started.
    { id: "children", quest: true, target: `[data-pane-tree="${CONDUCTOR}"]`, side: "right", gesture: { click: true }, shown: `[data-agent-child="${SCREEN}"]` },
    // The workers run until the Conductor has a question.
    { id: "running", target: "[data-mission]", side: "left", wait: true, done: (model) => model.stage >= 5 },
    // The question lands in 내 차례; the row opens the Conductor.
    { id: "needsyou", quest: true, target: `[data-task-row="${CONDUCTOR}"]`, side: "right", gesture: { click: true }, done: (model) => model.looked || model.stage >= 6 },
    // The map's banner takes the answer: a choice, or `직접 답하기…`; words said to the Conductor answer it too.
    { id: "answer", quest: true, target: ANSWER, side: "left", gesture: { click: true }, done: (model) => model.stage >= 6 },
    // What the answer changed runs to the end by itself: the workers finish and the Conductor reports.
    { id: "finishing", target: "[data-mission]", side: "left", wait: true, done: (model) => model.stage >= 9 },
    // ⇧⌘L opens Hide AI on its Memory tab; the rail icon would open it on 제안.
    { id: "memory", quest: true, target: '[data-lane-tool="memory"]', side: "left", command: "open_project_memory", done: showsMemory },
    // What Memory is, on the card the decision became; the last bubble says what to do next.
    { id: "kept", target: `[data-memory-card="${DECISION_CARD}"]`, side: "left" },
  ],
});
