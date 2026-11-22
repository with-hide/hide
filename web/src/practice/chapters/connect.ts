// Flow 9: another computer that runs Hide. The Mac mini shared a link; the
// learner opens Settings › Devices, pastes the link under Add device and
// presses Connect, checks that both screens show the same four words, and the
// two are connected. The Mac mini's tile comes up on the rail, and a few
// seconds later its own agent asks something: that computer's work, shown
// here with its name. The other computer is staged: a practice screen cannot
// reach one (`../worlds/connect.ts`).

import { chapter } from "../chapter";
import { CONNECT_WORLD, joined, LINK, MINI, miniAsks, paired, type ConnectModel } from "../worlds/connect";

export default chapter<ConnectModel>({
  id: "connect",
  number: 9,
  version: 1,
  since: "1.3.0",
  world: CONNECT_WORLD,
  quest: true,
  preview: "switch",
  steps: [
    { id: "settings", quest: true, target: "[data-device-rail]", side: "right", scene: 1500, command: "settings", shown: "[data-settings-tab]" },
    { id: "tab", quest: true, target: '[data-settings-tab="devices"]', side: "right", gesture: { click: true }, shown: "[data-device-add-open]" },
    { id: "add", quest: true, target: "[data-device-add-open]", side: "left", gesture: { click: true }, shown: "[data-add-link-input]" },
    // The link the Mac mini shared, in the field; done once the whole link is there, not when Connect turns on at its
    // first words, so a quick press never sends half a link.
    { id: "paste", target: "[data-add-by-link]", side: "left", gesture: { type: { input: "[data-add-link-input]", text: LINK, open: "[data-device-add-open]" } }, shown: `[data-add-link-input][value="${LINK}"]` },
    { id: "connect", quest: true, target: "[data-add-link-go]", side: "left", gesture: { click: true }, done: joined },
    // The same four words on both screens, in the same order.
    { id: "words", target: "[data-pair-words]", side: "left" },
    { id: "same", quest: true, target: "[data-pair-same]", side: "left", gesture: { click: true }, done: paired },
    // Done once the pairing window and Settings are both gone, so the rail shows.
    { id: "close", target: "[data-pair-join-close]", side: "left", gesture: { click: true }, shown: "body:not(:has([data-pair-join])) [data-device-rail]" },
    { id: "settingsClose", target: "[data-settings-close]", side: "left", gesture: { click: true }, shown: "body:not(:has([data-settings-close])) [data-device-rail]" },
    { id: "tile", target: `[data-rail-tile="${MINI.device}"]`, side: "right" },
    // The Mac mini's agent asks a few seconds after the two connect.
    { id: "asks", target: "[data-device-rail]", side: "right", wait: true, done: miniAsks },
    { id: "work", target: `[data-peer-row="${MINI.device}:w1:p1"]`, side: "right" },
    { id: "end", target: `[data-rail-tile="${MINI.device}"]`, side: "right" },
  ],
});
