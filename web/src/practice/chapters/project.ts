// Chapter 7: the right lane beside the agent: the project's counts and its week,
// the checkout's files and what changed in them, and the project's Memory.

import { chapter } from "../chapter";
import { laneDrawn, PROJECT_WORLD, showsTool, type ProjectModel } from "../worlds/project";

export default chapter<ProjectModel>({
  id: "project",
  number: 6,
  version: 3,
  world: PROJECT_WORLD,
  quest: true,
  preview: "lane",
  steps: [
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
