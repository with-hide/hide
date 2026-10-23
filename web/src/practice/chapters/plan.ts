// Chapter 7: asking an agent to plan a bigger change first, opening the plan
// from its pane's chip, reading it large in the 보기 slot, and approving it in
// the conversation, after which the agent starts on the first step.

import { chapter } from "../chapter";
import { LEAD, PLAN_WORLD, planInSlot, type PlanModel } from "../worlds/plan";

const PANE = `[data-pane-view="${LEAD}"]`;
const CHIP = `${PANE} [data-pane-plan-chip]`;
/** The small map's `크게 ›`, or, when the learner closed the map, the chip that opens it again. */
const LARGE = `[data-pane-plan-large], body:not(:has([data-pane-plan-large])) ${CHIP}`;
const say = (text: string) => ({ type: { pane: LEAD, text } });

export default chapter<PlanModel>({
  id: "plan",
  number: 7,
  version: 1,
  world: PLAN_WORLD,
  quest: true,
  steps: [
    { id: "intro", target: PANE, side: "inside" },
    // Any words ask: the agent puts its plan up for review before it changes anything.
    { id: "ask", quest: true, target: PANE, side: "inside", gesture: say("Make payment retries back off and read the count from settings. Plan it first and let me review it"), done: (model) => model.stage >= 1 },
    // Pointing at the chip, pressing it or the chord opens the small map; the shell opens it with no core event.
    { id: "chip", quest: true, target: CHIP, side: "bottom", command: "toggle_plan", shown: `[data-pane-plan="${LEAD}"]` },
    { id: "large", target: LARGE, side: "right", gesture: { click: true }, done: planInSlot },
    { id: "review", target: '[data-plan-banner="review"]', side: "left" },
    // Approval is a conversation: whatever the learner answers, the agent approves its plan and starts.
    { id: "approve", quest: true, target: PANE, side: "inside", gesture: say("Looks good. Cap the backoff at 30 seconds, then go"), done: (model) => model.stage >= 2 },
    { id: "end", target: CHIP, side: "bottom" },
  ],
});
