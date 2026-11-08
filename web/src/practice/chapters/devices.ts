// Flow 5: running work on more than one computer. The device rail's tiles, the
// mark that says a computer needs you, bringing the Mac mini forward, what
// its screen shows, opening a tab on it, and going back to this Mac.

import { chapter } from "../chapter";
import { backHere, DEVICES_WORLD, MINI, startedThere, visitedThere, type DevicesModel } from "../worlds/devices";

const TILE = `[data-rail-tile="${MINI}"]`;

export default chapter<DevicesModel>({
  id: "devices",
  number: 5,
  version: 1,
  world: DEVICES_WORLD,
  quest: true,
  preview: "devices",
  steps: [
    { id: "rail", target: "[data-device-rail]", side: "right" },
    // The tile's corner shows what waits on you there, before you open it.
    { id: "mark", target: TILE, side: "right" },
    { id: "tile", quest: true, target: TILE, side: "right", gesture: { click: true }, done: visitedThere },
    // The sidebar, the tabs and the terminals are now that computer's; the band names it.
    { id: "screen", target: "[data-agent-area]", side: "inside" },
    { id: "list", target: "[data-task-list]", side: "right" },
    // A new tab opens on that computer, so what you start there runs there.
    { id: "start", quest: true, target: "[data-agent-area]", side: "inside", command: "new_tab", done: startedThere },
    { id: "back", quest: true, target: '[data-rail-tile="local"]', side: "right", gesture: { click: true }, done: backHere },
    { id: "end", target: "[data-device-rail]", side: "right" },
  ],
});
