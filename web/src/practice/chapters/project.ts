// Flow 5: pinning a project in the sidebar, then the right lane beside the agent:
// the project's counts and its week, the checkout's files and what changed in
// them, and the project's Memory.

import { chapter } from "../chapter";
import { isPinned, laneDrawn, PIN_TARGET, PROJECT_WORLD, showsTool, type ProjectModel } from "../worlds/project";

export default chapter<ProjectModel>({
  id: "project",
  number: 5,
  version: 4,
  world: PROJECT_WORLD,
  quest: true,
  preview: "lane",
  steps: [
    // The pin waits for the pointer in the project row's buttons; pressed, the project moves to the top with a line under the pinned ones.
    { id: "pin", quest: true, target: `[data-row-pin="${PIN_TARGET}"]`, side: "right", gesture: { click: true }, done: (model) => isPinned(model, PIN_TARGET) },
    // The pinned project and the line that sets the others apart from it.
    { id: "pinned", target: '[data-project-pinned="true"], [data-sidebar] li[role="separator"]', side: "right" },
    // The Project tool's own icon, not ⌘E: ⌘E brings the lane back on whatever tool the learner last showed.
    { id: "lane", quest: true, target: '[data-lane-tool="project"]', side: "left", command: "toggle_lane_project", done: (model) => showsTool(model, "project") },
    { id: "counts", target: "[data-project-counts]", side: "left" },
    { id: "week", target: "[data-project-week]", side: "left" },
    { id: "explorer", quest: true, target: '[data-lane-tool="explorer"]', side: "left", command: "toggle_lane_explorer", done: (model) => showsTool(model, "explorer") },
    { id: "changes", target: "[data-explorer-tree]", side: "left" },
    // ⇧⌘L opens Hide AI on its Memory tab, where the next step's cards are; the rail icon would open it on 제안.
    { id: "memory", quest: true, target: '[data-lane-tool="memory"]', side: "left", command: "open_project_memory", done: (model) => showsTool(model, "memory") },
    { id: "cards", target: "[data-memory-cards]", side: "left" },
    { id: "close", quest: true, target: "[data-agent-area]", side: "inside", command: "toggle_lane", done: (model) => !laneDrawn(model) },
    { id: "end", target: "[data-tool-rail]", side: "left" },
  ],
});
