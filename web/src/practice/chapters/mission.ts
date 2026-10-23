// Chapter 8: handing a Conductor a job of several pull requests as a mission,
// opening its small map from the pane's mission chip, answering the one
// decision it asks in the conversation, and reading the mission's big map in
// the 보기 slot.

import { chapter } from "../chapter";
import { CONDUCTOR, MISSION_WORLD, missionInSlot, type MissionModel } from "../worlds/mission";

const PANE = `[data-pane-view="${CONDUCTOR}"]`;
const CHIP = `${PANE} [data-pane-mission-chip]`;
/** The small map's `크게 ›`, or, when the learner closed the map, the chip that opens it again. */
const LARGE = `[data-mission-mini-large], body:not(:has([data-mission-mini-large])) ${CHIP}`;
/** The small map's decision line, or, when the learner closed the map, the pane the answer is typed into. */
const DECISION = `[data-mission-mini-decision], body:not(:has([data-mission-mini-decision])) ${PANE}`;
const say = (text: string) => ({ type: { pane: CONDUCTOR, text } });

export default chapter<MissionModel>({
  id: "mission",
  number: 8,
  version: 1,
  world: MISSION_WORLD,
  quest: true,
  steps: [
    { id: "intro", target: PANE, side: "inside" },
    // Any words make the mission: the Conductor plans it, starts its workers, and asks one decision.
    { id: "ask", quest: true, target: PANE, side: "inside", gesture: say("/hide-conductor Make payment retries configurable with backoff and docs, as 3 PRs. Start once the plan is up"), done: (model) => model.stage >= 1 },
    // The chip and its small map, which a first plan that lands while the learner looks at the pane opens once,
    // pinned (missionReveal.ts). A look: ⏎ pressing the chord here would close a map the plan left pinned.
    { id: "chip", target: `${CHIP}, [data-pane-mission]`, side: "bottom", command: "toggle_plan" },
    // The decision is answered in the conversation: whatever the learner says, the Conductor changes the plan on it.
    { id: "answer", quest: true, target: DECISION, side: "bottom", gesture: say("Yes, fold the docs into the backoff PR"), done: (model) => model.stage >= 2 },
    { id: "large", quest: true, target: LARGE, side: "right", gesture: { click: true }, done: missionInSlot },
    { id: "map", target: "[data-mission]", side: "left" },
    { id: "end", target: CHIP, side: "bottom" },
  ],
});
