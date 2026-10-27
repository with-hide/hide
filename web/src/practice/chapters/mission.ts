// Chapter 8: handing a Conductor a job of several pull requests as a mission,
// reading its popover on the pane's mission chip, pressing the chip for the
// mission's map in the 보기 slot, answering the one decision it asks on the
// map, and reading the mission's flow.

import { chapter } from "../chapter";
import { CONDUCTOR, MISSION_WORLD, missionInSlot, type MissionModel } from "../worlds/mission";

const PANE = `[data-pane-view="${CONDUCTOR}"]`;
const CHIP = `${PANE} [data-pane-mission-chip]`;
/** The banner's recommended choice, or, while the map is not open, the chip that opens it. */
const ANSWER = `[data-mission-recommended], body:not(:has([data-mission-recommended])) ${CHIP}`;
const say = (text: string) => ({ type: { pane: CONDUCTOR, text } });

export default chapter<MissionModel>({
  id: "mission",
  number: 8,
  version: 3,
  world: MISSION_WORLD,
  quest: true,
  steps: [
    { id: "intro", target: PANE, side: "inside" },
    // Any words make the mission: the Conductor plans it, starts its workers, and asks one decision.
    { id: "ask", quest: true, target: PANE, side: "inside", gesture: say("/hide-conductor Make payment retries configurable with backoff and docs, as 3 PRs. Start once the plan is up"), done: (model) => model.stage >= 1 },
    // The chip and its popover, which a first plan that lands while the learner looks at the pane opens once,
    // pinned (missionReveal.ts). A look: ⏎ pressing the chord here would close a map the plan left pinned.
    { id: "chip", target: `${CHIP}, [data-pane-mission]`, side: "bottom", command: "toggle_plan" },
    { id: "large", quest: true, target: CHIP, side: "right", gesture: { click: true }, done: missionInSlot },
    // The banner on the map takes the answer: a choice, or `직접 답하기…`; words said to the Conductor answer it too.
    { id: "answer", quest: true, target: ANSWER, side: "left", gesture: { click: true }, done: (model) => model.stage >= 2 },
    { id: "map", target: "[data-mission]", side: "left" },
    { id: "end", target: CHIP, side: "bottom" },
  ],
});
