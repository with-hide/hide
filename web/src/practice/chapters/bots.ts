// Flow 8: bots. A program outside Hide that asked with `hide access request`
// waits in Settings › 봇; the learner opens it, reads the two modes, and
// approves the request in its own space. Its owl lands on the device rail
// beside a 전체 bot already connected, and that bot's agent then asks the
// learner in 내 차례, wearing the bot's owl. The request is staged: a practice
// screen cannot run an outside program (`../worlds/bots.ts`).

import { chapter } from "../chapter";
import { approved, BOT_PANE, BOTS_WORLD, botAsks, FULL, type BotsModel } from "../worlds/bots";

export default chapter<BotsModel>({
  id: "bots",
  number: 8,
  version: 1,
  since: "1.3.0",
  world: BOTS_WORLD,
  quest: true,
  preview: "rows",
  steps: [
    // The request is already there, as it would be once the program asked; Settings is where it is answered, and only there.
    { id: "settings", quest: true, target: "[data-device-rail]", side: "right", scene: 1500, command: "settings", shown: "[data-settings-tab]" },
    { id: "tab", quest: true, target: '[data-settings-tab="externalTools"]', side: "right", gesture: { click: true }, shown: "[data-access-pending]" },
    { id: "request", target: "[data-access-pending]", side: "left" },
    // The two modes: its own space is chosen, the narrower.
    { id: "modes", target: "[data-access-choice]", side: "left" },
    { id: "approve", quest: true, target: "[data-access-approve]", side: "left", gesture: { click: true }, done: approved },
    // The 전체 bot already connected: the lime ring, and its work is the learner's.
    { id: "full", target: `[data-access-connection="${FULL.connection}"]`, side: "left" },
    // Done once Settings is gone, so the rail shows again.
    { id: "close", target: "[data-settings-close]", side: "left", gesture: { click: true }, shown: "body:not(:has([data-settings-close])) [data-device-rail]" },
    { id: "rail", target: "[data-device-rail] [data-rail-tool]", side: "right" },
    // The 전체 bot's agent asks a few seconds after the approval.
    { id: "asks", target: "[data-device-rail]", side: "right", wait: true, done: botAsks },
    // A look: the owl on the row's logo, and its hint naming the bot.
    { id: "badge", target: `[data-agent-open="${BOT_PANE}"]`, side: "right" },
    { id: "end", target: "[data-device-rail] [data-rail-tool]", side: "right" },
  ],
});
