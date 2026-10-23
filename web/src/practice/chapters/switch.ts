// Chapter 2: going back and forth fast: the agent pane used before, the
// project used before, and a file found by name.

import { chapter } from "../chapter";
import { SWITCH_WORLD, type SwitchModel } from "../worlds/switch";

export default chapter<SwitchModel>({
  id: "switch",
  number: 2,
  version: 2,
  world: SWITCH_WORLD,
  quest: true,
  steps: [
    { id: "recent", quest: true, target: '[data-pane-view="pay"]', side: "inside", command: "recent_area_tab", done: (model) => model.left !== null },
    // Whichever agent the learner went back to, the project before is the other one.
    { id: "project", quest: true, target: "[data-sidebar-box] [data-project-list]", side: "right", command: "recent_project", done: (model) => model.left !== null && model.front !== model.left },
    { id: "file", quest: true, target: "[data-agent-area]", side: "inside", command: "open_file", done: (model) => model.index !== null },
    // While the palette is up it holds the keyboard and ⏎ there opens the file it highlights;
    // once Esc closed it, ⏎ presses the command again and the palette comes back.
    { id: "pick", target: "[data-palette]", side: "left", command: "open_file", done: (model) => model.opened !== null },
    { id: "end", target: "[data-lane-slot]", side: "left" },
  ],
});
