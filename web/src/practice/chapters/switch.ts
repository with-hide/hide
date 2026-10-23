// Chapter 2: going back and forth fast: the agent pane used before, the
// project used before, and a file found by name.

import { chapter } from "../chapter";
import { frontPane, SWITCH_WORLD, type SwitchModel } from "../worlds/switch";

export default chapter<SwitchModel>({
  id: "switch",
  number: 2,
  version: 1,
  world: SWITCH_WORLD,
  quest: true,
  steps: [
    { id: "recent", quest: true, target: '[data-pane-view="pay"]', side: "inside", command: "recent_area_tab", done: (model) => frontPane(model) === "login" },
    { id: "project", quest: true, target: "[data-sidebar-box] [data-project-list]", side: "right", command: "recent_project", done: (model) => model.front === "practice-docs" },
    { id: "file", quest: true, target: "[data-agent-area]", side: "inside", command: "open_file", done: (model) => model.index !== null },
    // The palette holds the keyboard: ⏎ there opens the file it highlights.
    { id: "pick", target: "[data-palette]", side: "left", done: (model) => model.opened !== null },
    { id: "end", target: "[data-lane-slot]", side: "left" },
  ],
});
