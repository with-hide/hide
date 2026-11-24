// Flow 10: answering from the phone. An agent asks which way to go while the
// learner is away from the desk; its row waits in 내 차례, and the same
// question stands on the learner's phone beside the screen, each choice with
// what the agent wrote under it. The learner marks a choice, sends it with
// 답하기, watches the agent's own screen take the keys the computer types for
// it, and sees the phone say the agent runs a command, by the clock.

import { chapter } from "../chapter";
import { ANSWER_WORLD, ASKING, answered, moved, picked, type AnswerModel } from "../worlds/answer";

const PHONE = "[data-practice-phone]";

export default chapter<AnswerModel>({
  id: "answer",
  number: 11,
  version: 1,
  since: "1.2.12",
  world: ANSWER_WORLD,
  quest: true,
  preview: "rows",
  steps: [
    // The row in 내 차례, as on any day: the question waits for the learner.
    { id: "row", target: `[data-agent-open="${ASKING}"]`, side: "right", scene: 1200 },
    // The same question on the phone, at the foot of the agent's conversation.
    { id: "phone", target: `${PHONE} [data-phone-ask]`, side: "left" },
    { id: "pick", quest: true, target: `${PHONE} [data-phone-ask-choice="2"]`, side: "left", gesture: { click: true }, done: picked },
    { id: "answer", quest: true, target: `${PHONE} [data-phone-ask-send]`, side: "left", gesture: { click: true }, done: answered },
    // The agent's own screen takes ↓ and Enter a moment later, and the agent goes on.
    { id: "keys", target: `[data-pane-view="${ASKING}"]`, side: "right", wait: true, done: moved },
    { id: "running", target: `${PHONE} [data-phone-activity]`, side: "left" },
    { id: "end", target: PHONE, side: "left" },
  ],
});
