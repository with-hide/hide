// Flow 3: handing a bigger change to an agent in steps. The agent shows its steps first and starts only
// after the learner says OK; the learner opens them from the pane's chip, answers in the conversation,
// and watches the agent work down the list, each step ticking.

import { chapter } from "../chapter";
import { LEAD, PLAN_WORLD, type PlanModel } from "../worlds/plan";

const PANE = `[data-pane-view="${LEAD}"]`;
const CHIP = `${PANE} [data-pane-plan-chip]`;
/** The words typed in the learner's language, a catalog key read when they are typed. */
const say = (words: string) => ({ type: { pane: LEAD, words } });

export default chapter<PlanModel>({
  id: "plan",
  number: 3,
  version: 3,
  world: PLAN_WORLD,
  quest: true,
  preview: "checklist",
  steps: [
    { id: "intro", target: PANE, side: "inside" },
    // Any words ask: the agent puts its plan up for review before it changes anything.
    { id: "ask", quest: true, target: PANE, side: "inside", gesture: say("practice.plan.ask.example"), done: (model) => model.stage >= 1 },
    // Pointing at the chip, pressing it or the chord opens the checklist; the shell opens it with no core event.
    { id: "chip", quest: true, target: CHIP, side: "bottom", command: "toggle_plan", shown: `[data-pane-plan="${LEAD}"]` },
    // Approval is a conversation: whatever the learner answers, the agent approves its plan and starts.
    { id: "approve", quest: true, target: PANE, side: "inside", gesture: say("practice.plan.approve.example"), done: (model) => model.stage >= 2 },
    // The agent works down the steps by itself; the chip follows the one it is on.
    { id: "ticking", target: CHIP, side: "bottom", wait: true, done: (model) => model.stage >= 4 },
    { id: "end", target: CHIP, side: "bottom" },
  ],
});
