// Flow 8: bots. The learner makes a bot in Settings › 봇: its name, and the
// mode, its own space kept. The core answers with the sentence for the AI that
// will use it, carrying a one-time code; the learner copies it and the bot
// waits for its AI, which joins a few seconds later (staged: a practice screen
// cannot run an outside AI, `../worlds/bots.ts`). Beside it a 전체 bot already
// connected wears a person in its owl's corner and has no rail tile; the made
// bot in its own space lands on the rail, and the 전체 bot's agent then asks
// the learner in 내 차례, wearing that bot's owl.

import { chapter } from "../chapter";
import { BOT_PANE, BOTS_WORLD, botAsks, copied, FULL, joined, MADE, made, SENT, type BotsModel } from "../worlds/bots";

const CREATE = "[data-access-create]";
const NAME = "[data-access-create-name]";

export default chapter<BotsModel>({
  id: "bots",
  number: 8,
  version: 2,
  since: "1.3.0",
  world: BOTS_WORLD,
  quest: true,
  preview: "rows",
  steps: [
    // Settings is where a bot is made, and only there.
    { id: "settings", quest: true, target: "[data-device-rail]", side: "right", scene: 1500, command: "settings", shown: "[data-settings-tab]" },
    { id: "tab", quest: true, target: '[data-settings-tab="externalTools"]', side: "right", gesture: { click: true }, shown: CREATE },
    { id: "create", quest: true, target: CREATE, side: "left", gesture: { click: true }, shown: "[data-access-create-dialog]" },
    // The name the AI will use in Hide, and the owl it gets from it; Esc closed the dialog, ⏎ opens it again.
    { id: "name", quest: true, target: NAME, side: "bottom", gesture: { type: { input: NAME, text: "nightly", open: CREATE } }, shown: "[data-access-create-submit]:not(:disabled)" },
    // The two modes: its own space is chosen, the narrower.
    { id: "modes", target: "[data-access-create-dialog] [data-access-choice]", side: "bottom" },
    { id: "make", quest: true, target: "[data-access-create-submit]", side: "left", gesture: { click: true }, done: made },
    { id: "sentence", target: "[data-access-sentence-text]", side: "bottom" },
    { id: "copy", quest: true, target: "[data-access-sentence-copy]", side: "left", gesture: { click: true }, done: copied },
    { id: "send", target: "[data-access-sentence-close]", side: "left", gesture: { click: true }, shown: "body:not(:has([data-access-sentence])) [data-access-invite]" },
    // The AI runs the command a few seconds after the sentence went.
    { id: "waits", target: "[data-access-invite]", side: "left", wait: true, enter: { kind: SENT, payload: {} }, done: joined },
    { id: "joined", target: `[data-access-connection="${MADE.id}"]`, side: "left" },
    // The 전체 bot already connected: the person in its owl's corner, and its work is the learner's.
    { id: "full", target: `[data-access-connection="${FULL.connection}"]`, side: "left" },
    // Done once Settings is gone, so the rail shows again.
    { id: "close", target: "[data-settings-close]", side: "left", gesture: { click: true }, shown: "body:not(:has([data-settings-close])) [data-device-rail]" },
    { id: "rail", target: "[data-device-rail]", side: "right" },
    // The 전체 bot's agent asks a few seconds after the made bot joined.
    { id: "asks", target: "[data-device-rail]", side: "right", wait: true, done: botAsks },
    // A look: the owl on the row's logo, and its hint naming the bot.
    { id: "badge", target: `[data-agent-open="${BOT_PANE}"]`, side: "right" },
    { id: "end", target: "[data-device-rail]", side: "right" },
  ],
});
