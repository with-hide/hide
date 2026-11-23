// Flow 12: files from the phone. An agent reported while the learner was away;
// its conversation stands on the learner's phone beside the screen. The learner
// presses the `+` at the start of the reply field, sees a screenshot and a log
// stand on the bar while a file past 10 MB is left out with one line, waits for
// the log to be in, sends the words and the files as one message, and watches
// the agent's own screen take the files' paths and read them.

import { chapter } from "../chapter";
import { ATTACHMENTS_WORLD, MEASURING, answered, picked, sent, uploaded, type AttachmentsModel } from "../worlds/attachments";

const PHONE = "[data-practice-phone]";

export default chapter<AttachmentsModel>({
  id: "attachments",
  number: 12,
  version: 1,
  since: "1.3.0",
  world: ATTACHMENTS_WORLD,
  quest: true,
  preview: "rows",
  steps: [
    // The agent's report on the phone, as on any day away from the desk.
    { id: "phone", target: PHONE, side: "left", scene: 1200 },
    { id: "attach", quest: true, target: `${PHONE} [data-phone-attach]`, side: "left", gesture: { click: true }, done: picked },
    // The photo stands as its thumbnail, the log as its chip with its band; the trace past 10 MB is left out with one line.
    { id: "files", target: `${PHONE} [data-phone-attachments]`, side: "left", wait: true, done: uploaded },
    { id: "send", quest: true, target: `${PHONE} [data-phone-send]`, side: "left", gesture: { click: true }, done: sent },
    // The agent's own screen takes the files' paths before the words, and reads them.
    { id: "typed", target: `[data-pane-view="${MEASURING}"]`, side: "right", wait: true, done: answered },
    { id: "end", target: PHONE, side: "left" },
  ],
});
