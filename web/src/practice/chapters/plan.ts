// Flow 3: asking an agent for the steps of a bigger change first, opening them
// from its pane's chip, reading them large in the 보기 slot, and approving them in
// the conversation, after which the agent works through the steps and each ticks.

import { chapter } from "../chapter";
import { LEAD, PLAN_WORLD, planInSlot, type PlanModel } from "../worlds/plan";

const PANE = `[data-pane-view="${LEAD}"]`;
const CHIP = `${PANE} [data-pane-plan-chip]`;
/** The checklist's `크게 ›`, or, when the learner closed it, the chip that opens it again. */
const LARGE = `[data-pane-plan-large], body:not(:has([data-pane-plan-large])) ${CHIP}`;
const say = (text: string) => ({ type: { pane: LEAD, text } });

export default chapter<PlanModel>({
  id: "plan",
  number: 3,
  version: 2,
  world: PLAN_WORLD,
  quest: true,
  preview: "checklist",
  steps: [
    { id: "intro", target: PANE, side: "inside" },
    // Any words ask: the agent puts its plan up for review before it changes anything.
    { id: "ask", quest: true, target: PANE, side: "inside", gesture: say("Make payment retries back off and read the count from settings. Use hide-task to plan the steps first and let me review them"), done: (model) => model.stage >= 1 },
    // Pointing at the chip, pressing it or the chord opens the checklist; the shell opens it with no core event.
    { id: "chip", quest: true, target: CHIP, side: "bottom", command: "toggle_plan", shown: `[data-pane-plan="${LEAD}"]` },
    { id: "large", target: LARGE, side: "right", gesture: { click: true }, done: planInSlot },
    { id: "review", target: '[data-plan-banner="review"]', side: "left" },
    // Approval is a conversation: whatever the learner answers, the agent approves its plan and starts.
    { id: "approve", quest: true, target: PANE, side: "inside", gesture: say("Looks good. Cap the backoff at 30 seconds, then go"), done: (model) => model.stage >= 2 },
    // The agent works down the steps by itself; the chip follows the one it is on.
    { id: "ticking", target: CHIP, side: "bottom", wait: true, done: (model) => model.stage >= 4 },
    { id: "end", target: CHIP, side: "bottom" },
  ],
});
